// 应用内检查更新（tauri-plugin-updater）：
// - 仅手动触发（设置页 about tab「软件更新」分组），无自动检查、无持久化设置项
// - 静态 JSON endpoint（GitHub Releases latest.json），Ed25519 签名验证不可关闭
// - 代理策略（设计稿 §4.3）：复用既有 proxy_mode/proxy_url——
//   none → 直连；system → 走系统代理；custom → 显式走 proxy_url。
//
// 检查走自建的 `updater_check` 命令（src-tauri/src/commands/updater.rs），
// 不用插件的 `check()`：插件 JS API 的 CheckOptions 只有 proxy、没有表达「强制直连」
// 的字段，而插件内部用 `reqwest::ClientBuilder::new()` 建客户端（默认
// auto_sys_proxy: true），proxy 为 undefined 时会追加系统代理——那么 proxy_mode=none
// 在检查更新上就是假的，与后台监控（http.rs 对 none 调 no_proxy()）语义相反。
//
// 下载与安装仍走插件自带命令（只接受 headers/timeout）：proxy 随 check 时构建的
// Update 资源贯穿到下载阶段，无需（也不能）重复传。
import { computed, ref, shallowRef, watch } from 'vue'
import { Update, type DownloadEvent } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { confirm } from '@tauri-apps/plugin-dialog'
import { invokeI18nFn, openReleaseUrl } from './client'
import { commands, type UpdaterMetadata } from '../bindings'
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
  // ReleaseNotFound 是插件对「endpoint 未给出可用 release」的统一兜底（updater.rs）：
  // 404（该版本还没有 latest.json）、403/500（GitHub 限流或服务端故障）、JSON 解析失败
  // 全都落这一种，因此本类的实际含义比文案「检查更新失败」更宽——它也可能只是
  // 本版本还没有 latest.json（旧版用户），并非真的出了故障。UI 上重试与打开下载页
  // 两个入口并存（SettingsTab.showUpdate*），两条出路都不堵，故维持单一归类。
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
/**
 * @param onLogWritten 更新链路每次写完操作日志后回调（落库在 Rust 端，前端 fire-and-forget
 *   触发）——宿主用它重拉日志列表，否则日志页是 v-show 常驻的，看不到新写的日志。
 */
export function useAppUpdate(
  getProxy: () => { mode: string; url: string },
  onLogWritten?: () => void,
) {
  const status = ref<UpdateStatus>('idle')
  const currentVersion = ref('')
  // pendingUpdate 必须是 shallowRef：插件 Update 实例内部用 JS 私有字段（#metadata 等），
  // 若用深度 ref 会被 Vue 包成 reactive Proxy，调用 downloadAndInstall() 时 this 是 Proxy，
  // 访问私有字段会抛 "Cannot read private member from an object whose class did not declare it"。
  // （单元测试 mock 的 Update 类无私有字段，因此测试全绿但没暴露此问题。）
  const pendingUpdate = shallowRef<Update | null>(null)
  const errorKind = ref<UpdateErrorKind>('generic')
  /** generic 类错误透传的原始消息（update.error.generic 的 {message}） */
  const errorDetail = ref('')
  const done = ref(0)
  const total = ref<number | undefined>(undefined)
  /** error 态的重试去向：检查失败 → 重跑 check；下载失败 → 回 available */
  const retryTarget = ref<'check' | 'download'>('check')
  /** Release Note 弹窗开关（SettingsTab 据此挂载 UpdateNotesModal） */
  const showNotes = ref(false)

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

  /** 把后端返回的 UpdaterMetadata 转成插件 Update 的构造入参：
   *  rawJson 经字符串传递（见 updater.rs 注释）需 parse 还原；
   *  null 字段收敛为 undefined，对齐 UpdateMetadata 的可选字段类型。 */
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
      // 代理三态在 Rust 侧解释：none → no_proxy()，custom → proxy(url)，system → 系统代理。
      // 走 invokeI18nFn：err.* 类错误（如无效代理 URL）翻译为用户可读文案，
      // 非 err.*（网络/签名/ReleaseNotFound）保持英文原文，供 classifyUpdateError 按锚点归类。
      const { mode, url } = getProxy()
      const meta = await invokeI18nFn(() => commands.updaterCheck(CHECK_TIMEOUT_MS, mode, url))
      // 替换旧 Update 前先 close：每次 check 都会在 webview 资源表新增一个 rid，
      // 不 close 会随检查次数累积（与插件自身 check 行为一致，这里主动收敛）。
      pendingUpdate.value?.close()
      pendingUpdate.value = meta ? new Update(toUpdateMetadata(meta)) : null
      status.value = meta ? 'available' : 'upToDate'
    } catch (e) {
      handleError(e)
    }
    // 成功/失败均已各写一条检查日志，通知宿主重拉日志列表
    onLogWritten?.()
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
    // 写「开始下载」操作日志（Rust 端落库，失败不阻塞下载）；写完通知宿主重拉日志列表
    commands.updaterDownloadStarted(u.version).catch(() => null).finally(() => onLogWritten?.())
    try {
      // proxy 随 updater_check 时构建的 Update 资源生效，下载阶段不重复传（见文件头说明）
      await u.downloadAndInstall(onProgress)
      // Windows：NSIS 安装器 ShellExecuteW 成功后进程 exit(0) 接管，不会执行到这里；
      // 到达此处的仅 Linux/macOS：先优雅关闭 pi RPC（失败不阻塞重启），再 relaunch（§6）
      status.value = 'installing'
      commands.updaterInstallStarted(u.version).catch(() => null).finally(() => onLogWritten?.())
      await commands.agentShutdownForUpdate().catch(() => null)
      await relaunch()
    } catch (e) {
      // 写「下载失败」操作日志（Rust 端落库，失败不阻塞错误处理）
      const raw = e instanceof Error ? e.message : String(e)
      commands.updaterDownloadFailed(u.version, raw).catch(() => null).finally(() => onLogWritten?.())
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

  /** 弹窗内展示的 Release Note（latest.json 的 notes）；无 body 时为 null（不弹窗） */
  const notesBody = computed(() => pendingUpdate.value?.body ?? null)
  /** 新版本的构建日期（latest.json 的 pub_date），可能为空 */
  const notesVersion = computed(() => pendingUpdate.value?.version ?? '')
  const notesDate = computed(() => pendingUpdate.value?.date ?? null)

  /** 浏览器打开 GitHub Release 页：带安装包、commit 与历史版本，
   *  是弹窗内 Markdown 覆盖不到的部分。 */
  function openReleasePage(): void {
    const v = pendingUpdate.value?.version
    if (v) void openReleaseUrl(`https://github.com/hhelibeb/relwatch/releases/tag/v${v}`)
  }

  /** 「查看 Release 说明」：有 notes 走应用内弹窗；
   *  notes 缺失（latest.json 未写 notes，见发布流程）降级为浏览器打开 Release 页——
   *  此时弹一个空框不如不弹，但用户仍要看得见更新内容。 */
  function openReleaseNotes(): void {
    if (notesBody.value) {
      showNotes.value = true
      return
    }
    openReleasePage()
  }

  /** 离开 available 态（开始下载/重新检查/出错）时收起弹窗，避免残留旧版本内容。
   *  刻意不做「下载失败 → retry 回 available 后自动重开弹窗」：下载失败已进 error 态，
   *  用户主动 retry 后需要重新点「查看 Release 说明」——避免看到与已重置状态不符的旧版
   *  Release Note，也让用户对「重来一次」有明确心智。 */
  watch(status, (s) => {
    if (s !== 'available') showNotes.value = false
  })

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
