import type { App } from 'vue'
import { commands } from '../bindings'
import { t } from '../i18n'

/**
 * 前端全局错误兜底（V2）。
 *
 * ## 为什么需要它
 *
 * release 版没有控制台；未捕获的异常 / promise rejection 在此之前**三处皆无痕迹**：
 * 界面无提示、控制台不可见、日志表无记录。用户看到的只有「点了没反应」，报障时
 * 连线索都提供不了。本模块把这类异常变成「一条 toast + 一行可搜索的日志」。
 *
 * ## 分工
 *
 * - 前端（本模块）：捕获 → 节流 → 截断 → toast → 调 Rust 命令落库；
 * - 后端（`commands::report_frontend_error`）：写入 `logs` 表（经 `db::logs::write_log_key`
 *   默认脱敏，DB 失败时降级写 `logs/fallback.log`，见 V23/V28）。
 *
 * ## 命名约束（勿改）
 *
 * 上报 key **不得以 `err.` 开头**：`src/__tests__/i18n-keys.test.ts` 会扫 Rust 生产代码里
 * 的 `"err.xxx` 字面量并要求两个字典都有翻译，用 `err.` 前缀会把一个「前端键」误算成
 * 后端错误键。现用 `ui.vue_error` / `ui.unhandled_rejection` / `ui.window_error`。
 */

/** 同一 key 的上报节流窗口：错误风暴（如 1.5s 轮询里每次都抛）只留首条。 */
const THROTTLE_MS = 60_000
/** 落库明细长度上限：堆栈可能很长，避免单条日志把表撑爆。 */
const MAX_DETAIL_LEN = 2048
/** toast 里展示的明细长度上限：够定位即可，完整内容看日志页。 */
const MAX_TOAST_DETAIL_LEN = 160
/** Vue 错误处理器附带的 info（生命周期钩子名等）长度上限。 */
const MAX_INFO_LEN = 512

const lastReportedAt = new Map<string, number>()
let toastSink: ((message: string) => void) | null = null

/**
 * 注入 toast 出口。`main.ts` 的兜底处理器不在组件树内，拿不到 `provide/inject`，
 * 故由 `App.vue` 在 setup 期把 `showToast` 注册进来。
 */
export function setErrorToastSink(sink: (message: string) => void): void {
  toastSink = sink
}

/** 清空节流记录（仅测试用：模块级状态在用例之间不会自动重置）。 */
export function resetReportThrottle(): void {
  lastReportedAt.clear()
}

/** 把任意 reject 值 / 异常对象转成可落库的文本（Error 优先取 stack，便于定位）。 */
function describe(detail: unknown): string {
  if (detail instanceof Error) {
    return detail.stack || `${detail.name}: ${detail.message}`
  }
  if (typeof detail === 'string') return detail
  try {
    return JSON.stringify(detail) ?? String(detail)
  } catch {
    return String(detail)
  }
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text
}

/**
 * toast 文案：与日志共用同一条 i18n 键（模板形如 `…{error}`，Rust 侧 `render` 按命名
 * 占位符填充）。前端此处做同样的替换，否则用户会看到裸的 `{error}` 占位符。
 */
function toastText(key: string, detail: string): string {
  return t(key).replace('{error}', truncate(detail, MAX_TOAST_DETAIL_LEN))
}

/**
 * 上报一次前端异常：节流窗口内同 key 只记一次（toast 与日志同步去重）。
 *
 * 本函数**自身绝不抛错**：它被注册在 `unhandledrejection` 上，一旦抛出会再触发一次
 * `unhandledrejection`，形成无限递归。
 */
export async function reportFrontendError(
  key: string,
  detail: unknown,
  info?: string,
): Promise<void> {
  const now = Date.now()
  const last = lastReportedAt.get(key)
  if (last !== undefined && now - last < THROTTLE_MS) return
  lastReportedAt.set(key, now)

  const text = describe(detail)

  try {
    toastSink?.(toastText(key, text))
  } catch {
    // toast 出口自身异常不得影响上报
  }

  try {
    await commands.reportFrontendError(
      key,
      truncate(text, MAX_DETAIL_LEN),
      info ? truncate(info, MAX_INFO_LEN) : null,
    )
  } catch {
    // 上报失败不得递归；DB 侧失败还有 fallback.log 兜底（V23）
  }
}

/**
 * 挂载全局兜底处理器。必须在 `app.mount()` **之前**调用——渲染期的错误也要被捕获。
 *
 * 三个入口对应三类原本静默的失败：
 * - `app.config.errorHandler`：Vue 组件内同步异常（渲染 / 生命周期 / watcher）；
 * - `unhandledrejection`：未被 catch 的 Promise（含 async 事件回调里抛出的）；
 * - `window error`：Vue 之外的同步脚本错误（如模块顶层、原生事件回调）。
 */
export function installGlobalErrorHandlers(app: App): void {
  app.config.errorHandler = (err, _instance, info) => {
    // ⚠️ 安装 errorHandler 会**接管** Vue 原有的错误输出通道，必须在此把 dev 可见性补回来。
    //
    // Vue（`runtime-core` 的 `handleError`）的逻辑是：
    //     if (instance) { ...; if (errorHandler) { callWithErrorHandling(errorHandler, ...); return } }
    //     logError(err, type, contextVNode, throwInDev)
    // 即：一旦设置了 errorHandler 就**直接 return**，后面的 `logError` 不再执行。
    // 而 `logError` 在 dev 下正是负责 `warn('Unhandled error during execution of ...')`
    // 与 `throw err`（默认 `throwInDev=true`，让 dev 直接崩出来、堆栈落在 devtools）的那个函数。
    // 二者都被跳过 ⇒ 仅 toast + 日志页可查，**dev 控制台再无堆栈**——对正在自调试的人
    // 是实打实的退化（本次差点带着这个回归合并）。故显式补一条 console.error。
    //
    // 只限 DEV：`console.error` 本就在 prod 无控制台可看，且不希望在 release 里多一份输出。
    if (import.meta.env.DEV) {
      console.error(`[RelWatch] 未处理异常${info ? ` during execution of ${info}` : ''}`, err)
    }
    void reportFrontendError('ui.vue_error', err, info)
  }

  window.addEventListener('unhandledrejection', (event) => {
    void reportFrontendError('ui.unhandled_rejection', event.reason)
  })

  window.addEventListener('error', (event) => {
    // 资源加载失败（<img>/<script>）也触发 error 事件但 `error` 为 null——本应用的
    // 远程图片全部经 media 代理加载，不能把图片失败一律当「未处理异常」。
    if (!event.error) return
    void reportFrontendError('ui.window_error', event.error)
  })
}
