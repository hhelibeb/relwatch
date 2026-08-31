// 应用内检查更新（tauri-plugin-updater）：
// - 仅手动触发（设置页 general tab「软件更新」分组），无自动检查、无持久化设置项
// - 静态 JSON endpoint（GitHub Releases latest.json），Ed25519 签名验证不可关闭
// - 代理策略（设计稿 §4.3）：复用既有 proxy_mode/proxy_url——
//   none → 直连；system → 不传 proxy（Windows/macOS 走系统代理，Linux 等同直连）；
//   custom → 显式传 proxy_url。
//   注意（以插件 2.10.1 实际 API 核对）：proxy 只能在 check() 传入，
//   download/download_and_install 的 Rust 命令只接受 headers/timeout——
//   check() 配置的 proxy 随 Update 资源贯穿到下载阶段，无需（也不能）重复传。
import { computed, ref } from 'vue'
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { confirm } from '@tauri-apps/plugin-dialog'
import { openReleaseUrl } from './client'
import { commands } from '../bindings'
import { t, tm } from '../i18n'

export type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'upToDate'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'error'

/** §4.5 错误归类 kind（决定文案 key 与兜底动作按钮） */
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

/**
 * 插件错误 Display 文案锚点表（设计稿 §4.5）。
 * 锚点按 tauri-plugin-updater 2.10.1 的 error.rs Display 字符串与
 * minisign-verify 0.2.5 的 Display 实现逐一核对（签名错误来自
 * minisign 透明错误 + SignatureUtf8，锚点统一取 "signature"/"minisign"）。
 * 升级插件版本时须回归本表；后续若漂移再下沉 Rust 侧包结构化错误码。
 */
export function classifyUpdateError(raw: string): UpdateErrorKind {
  const m = raw.toLowerCase()
  // 顺序敏感：具体变体在前，兜底网络/泛化在后
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

/** total 单独存 ref：插件只在 Started 事件给一次 contentLength，
 * Progress 事件只有 chunkLength——不要把 total 传成 undefined 冲掉它（设计稿 §4.3）。 */
export function useAppUpdate(getProxy: () => { mode: string; url: string }) {
  const status = ref<UpdateStatus>('idle')
  const currentVersion = ref('')
  const pendingUpdate = ref<Update | null>(null)
  const errorKind = ref<UpdateErrorKind>('generic')
  /** generic 类错误透传的原始消息（update.error.generic 的 {message}） */
  const errorDetail = ref('')
  const done = ref(0)
  const total = ref<number | undefined>(undefined)
  /** error 态的重试去向：检查失败 → 重跑 check；下载失败 → 回 available */
  const retryTarget = ref<'check' | 'download'>('check')

  // getVersion 失败（非 Tauri 环境等）静默兜底，版本号显示为空即可
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

  /** 下载状态行：total 已知时带百分比与总量，未知时只报已下载（不显示百分比，§8） */
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

  /** 代理映射：none/system → undefined（直连或系统代理），custom → 显式 proxy_url */
  function resolveProxy(): string | undefined {
    const { mode, url } = getProxy()
    return mode === 'custom' && url ? url : undefined
  }

  function onProgress(e: DownloadEvent) {
    if (e.event === 'Started') {
      total.value = e.data.contentLength
    } else if (e.event === 'Progress') {
      done.value += e.data.chunkLength
    }
  }

  function handleError(e: unknown) {
    const raw = e instanceof Error ? e.message : String(e)
    errorKind.value = classifyUpdateError(raw)
    errorDetail.value = raw
    status.value = 'error'
  }

  async function checkForUpdate(): Promise<void> {
    if (busy.value) return
    status.value = 'checking'
    errorDetail.value = ''
    retryTarget.value = 'check'
    try {
      const found = await check({ timeout: CHECK_TIMEOUT_MS, proxy: resolveProxy() })
      pendingUpdate.value = found
      status.value = found ? 'available' : 'upToDate'
    } catch (e) {
      handleError(e)
    }
  }

  async function downloadAndInstall(): Promise<void> {
    const u = pendingUpdate.value
    if (!u || busy.value) return
    // Agent 任务守卫（设计稿 §4.3）：安装会硬杀进程，运行中任务与已消耗词元会实际丢失。
    // 队列查询失败时降级放行（后端不可用不该堵死更新路径）
    try {
      const queue = await commands.getAgentQueue()
      if (queue.length > 0) {
        const ok = await confirm(t('update.error.agent_busy'), {
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
    try {
      // proxy 随 check() 时构建的 Update 资源生效，下载阶段不重复传（见文件头说明）
      await u.downloadAndInstall(onProgress)
      // Windows：NSIS 安装器 ShellExecuteW 成功后进程 exit(0) 接管，不会执行到这里；
      // 到达此处的仅 Linux/macOS：先优雅关闭 pi RPC（失败不阻塞重启），再 relaunch（§6）
      status.value = 'installing'
      await commands.agentShutdownForUpdate().catch(() => null)
      await relaunch()
    } catch (e) {
      handleError(e)
    }
  }

  /** error 态重试：检查失败 → 重跑 check；下载失败 → 回 available（保留 Update 对象，§4.3 状态机） */
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

  function openReleaseNotes(): void {
    const v = pendingUpdate.value?.version
    if (v) void openReleaseUrl(`https://github.com/hhelibeb/relwatch/releases/tag/v${v}`)
  }

  function openDownloadPage(): void {
    void openReleaseUrl(DOWNLOAD_PAGE_URL)
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
    checkForUpdate,
    downloadAndInstall,
    retry,
    openReleaseNotes,
    openDownloadPage,
  }
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`
  if (n >= 1024) return `${Math.round(n / 1024)} KB`
  return `${n} B`
}
