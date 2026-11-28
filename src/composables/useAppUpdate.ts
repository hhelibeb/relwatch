// 应用内检查更新（tauri-plugin-updater）：
// - 仅手动触发（设置页 about tab「软件更新」分组），无自动检查、无持久化设置项
// - 静态 JSON endpoint（GitHub Releases latest.json），Ed25519 签名验证不可关闭
// - 代理策略：复用既有 proxy_mode/proxy_url（检查走自建 `updater_check` 命令以支持强制直连）
import { computed, getCurrentScope, onScopeDispose, ref, shallowRef, watch } from 'vue'
import { Update, type DownloadEvent } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { invokeI18nFn, openReleaseUrl } from '../api/client'
import { commands, type UpdaterMetadata } from '../bindings'
import { confirmAction, extractErrorMessage } from '../services/feedback'
import { t, tm } from '../i18n'

export type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'upToDate'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'error'

export type UpdateErrorKind =
  | 'network'
  | 'no_release'
  | 'signature'
  | 'targets'
  | 'format'
  | 'mount'
  | 'unsupported'
  | 'generic'

const CHECK_TIMEOUT_MS = 30_000
const DOWNLOAD_PAGE_URL = 'https://github.com/hhelibeb/relwatch/releases/latest'

export function classifyUpdateError(raw: string): UpdateErrorKind {
  const m = raw.toLowerCase()
  if (m.includes('could not fetch a valid release json')) return 'no_release'
  if (m.includes('signature') || m.includes('minisign')) return 'signature'
  if (m.includes('was not found in the response') || m.includes('none of the fallback platforms')) return 'targets'
  if (m.includes('invalid updater')) return 'format'
  if (m.includes('same mount point')) return 'mount'
  if (
    m.includes('unsupported os')
    || m.includes('unsupported application architecture')
    || m.includes('does not have any endpoints set')
  ) {
    return 'unsupported'
  }
  const networkAnchors = [
    'error sending request',
    'timed out',
    'timeout',
    'connection',
    'network',
    'dns',
    'tls',
    'ssl',
    'certificate',
    'temporarily unavailable',
  ]
  if (networkAnchors.some(a => m.includes(a))) return 'network'
  return 'generic'
}

export function useAppUpdate(
  getProxy: () => { mode: string; url: string },
  onLogWritten?: () => void,
) {
  const status = ref<UpdateStatus>('idle')
  const currentVersion = ref('')
  // shallowRef 避免 Vue reactive Proxy 导致插件 Update 类私有字段访问报错
  const pendingUpdate = shallowRef<Update | null>(null)
  const errorKind = ref<UpdateErrorKind>('generic')
  const errorDetail = ref('')
  const done = ref(0)
  const total = ref<number | undefined>(undefined)
  const retryTarget = ref<'check' | 'download'>('check')
  const showNotes = ref(false)

  void getVersion()
    .then(v => {
      currentVersion.value = v
    })
    .catch(() => null)

  const busy = computed(
    () => status.value === 'checking' || status.value === 'downloading' || status.value === 'installing',
  )

  const percent = computed(() => {
    if (total.value === undefined || total.value <= 0) return null
    return Math.min(100, Math.round((done.value / total.value) * 100))
  })

  const downloadText = computed(() => {
    const doneStr = formatBytes(done.value)
    if (total.value !== undefined && total.value > 0) {
      return tm('update.downloading', {
        percent: `${percent.value ?? 0}%`,
        done: doneStr,
        total: formatBytes(total.value),
      })
    }
    return tm('update.downloading_no_total', { done: doneStr })
  })

  const errorText = computed(() => {
    switch (errorKind.value) {
      case 'network':
        return t('update.error.network')
      case 'signature':
        return t('update.error.signature')
      case 'no_release':
        return t('update.error.no_release')
      case 'unsupported':
        return t('update.error.unsupported')
      default:
        return tm('update.error.generic', { message: errorDetail.value })
    }
  })

  function toUpdateMetadata(m: UpdaterMetadata) {
    return {
      rid: m.rid,
      currentVersion: m.currentVersion,
      version: m.version,
      date: m.date ?? undefined,
      body: m.body ?? undefined,
      rawJson: JSON.parse(m.rawJson) as Record<string, unknown>,
    }
  }

  function onProgress(e: DownloadEvent) {
    if (e.event === 'Started') {
      total.value = e.data.contentLength
    } else if (e.event === 'Progress') {
      done.value += e.data.chunkLength
    }
  }

  function handleError(e: unknown) {
    const raw = extractErrorMessage(e)
    errorKind.value = classifyUpdateError(raw)
    errorDetail.value = raw
    status.value = 'error'
  }

  let installGuard = false

  async function checkForUpdate(): Promise<void> {
    if (busy.value || installGuard) return
    status.value = 'checking'
    errorDetail.value = ''
    retryTarget.value = 'check'
    try {
      const { mode, url } = getProxy()
      const meta = await invokeI18nFn(() => commands.updaterCheck(CHECK_TIMEOUT_MS, mode, url))
      void Promise.resolve(pendingUpdate.value?.close()).catch(() => {})
      pendingUpdate.value = meta ? new Update(toUpdateMetadata(meta)) : null
      status.value = meta ? 'available' : 'upToDate'
    } catch (e) {
      handleError(e)
    }
    onLogWritten?.()
  }

  async function downloadAndInstall(): Promise<void> {
    const u = pendingUpdate.value
    if (!u || busy.value || installGuard) return
    installGuard = true
    try {
      try {
        const queue = await commands.getAgentQueue()
        if (queue.length > 0) {
          const ok = await confirmAction(t('update.error.agent_busy'), {
            title: t('update.section_title'),
            kind: 'warning',
          })
          if (!ok) return
        }
      } catch {
        // 降级：守卫查询失败不阻塞更新
      }
      status.value = 'downloading'
      done.value = 0
      total.value = undefined
      retryTarget.value = 'download'
      commands.updaterDownloadStarted(u.version).catch(() => null).finally(() => onLogWritten?.())
      try {
        await u.downloadAndInstall(onProgress)
        status.value = 'installing'
        commands.updaterInstallStarted(u.version).catch(() => null).finally(() => onLogWritten?.())
        await commands.agentShutdownForUpdate().catch(() => null)
        await relaunch()
      } catch (e) {
        const raw = extractErrorMessage(e)
        commands.updaterDownloadFailed(u.version, raw).catch(() => null).finally(() => onLogWritten?.())
        handleError(e)
      }
    } finally {
      installGuard = false
    }
  }

  function retry(): void {
    if (retryTarget.value === 'check') {
      void checkForUpdate()
      return
    }
    if (pendingUpdate.value) {
      status.value = 'available'
    } else {
      void checkForUpdate()
    }
  }

  const notesBody = computed(() => pendingUpdate.value?.body ?? null)
  const notesVersion = computed(() => pendingUpdate.value?.version ?? '')
  const notesDate = computed(() => pendingUpdate.value?.date ?? null)

  function openReleasePage(): void {
    const v = pendingUpdate.value?.version
    if (v) void openReleaseUrl(`https://github.com/hhelibeb/relwatch/releases/tag/v${v}`)
  }

  function openReleaseNotes(): void {
    if (notesBody.value) {
      showNotes.value = true
      return
    }
    openReleasePage()
  }

  watch(status, (s) => {
    if (s !== 'available') showNotes.value = false
  })

  function openDownloadPage(): void {
    void openReleaseUrl(DOWNLOAD_PAGE_URL)
  }

  if (getCurrentScope()) {
    onScopeDispose(() => {
      void Promise.resolve(pendingUpdate.value?.close()).catch(() => {})
      pendingUpdate.value = null
    })
  }

  return {
    status,
    currentVersion,
    pendingUpdate,
    errorKind,
    errorText,
    done,
    total,
    percent,
    downloadText,
    busy,
    showNotes,
    notesVersion,
    notesDate,
    notesBody,
    checkForUpdate,
    downloadAndInstall,
    retry,
    openReleaseNotes,
    openReleasePage,
    openDownloadPage,
  }
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`
  if (n >= 1024) return `${Math.round(n / 1024)} KB`
  return `${n} B`
}
