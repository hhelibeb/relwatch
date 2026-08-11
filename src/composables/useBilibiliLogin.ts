import { ref, onUnmounted } from 'vue'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import {
  readBilibiliLoginCookie,
  closeBilibiliLoginWindow,
  setCredential,
} from '../api/settings'
import { InvokeI18nError } from '../api/client'
import { t } from '../i18n'
import { track } from './useUsageTracking'

/**
 * B 站一键登录（应用内 WebView 扫码，自动读取 SESSDATA）完整状态机：
 * 窗口创建/复用、2s 登录态轮询、60s 超时保护、单调代次令牌防旧回调、
 * 窗口缺失/持续性错误的分支处理、卸载清理。
 *
 * 从 SettingsTab 抽出的纯状态机：表单凭据状态的更新通过回调交给调用方
 * （onLoginSuccess / onCookieCleared），UI 提示通过 showToast 注入。
 */
export function useBilibiliLogin(opts: {
  showToast: (msg: string) => void
  /** 登录成功：刷新表单凭据状态（如 form.bilibili_cookie_set = true） */
  onLoginSuccess: () => void
  /** Cookie 被清除：刷新表单凭据状态（如 form.bilibili_cookie_set = false） */
  onCookieCleared: () => void
}) {
  const { showToast, onLoginSuccess, onCookieCleared } = opts
  const biliLoginBusy = ref(false)
  let biliLoginPollTimer: ReturnType<typeof setInterval> | null = null
  let biliLoginTimeout: ReturnType<typeof setTimeout> | null = null
  /** 单调代次令牌：每次发起登录尝试递增，回调（轮询/超时）校验令牌后才生效，
   *  避免旧流程残留的异步回调误操作新流程（F3）。 */
  let biliLoginAttempt = 0
  let biliLoginSettled = false
  const BILI_LOGIN_WINDOW_LABEL = 'bilibili-login'

  function stopBiliLoginPolling() {
    if (biliLoginPollTimer) {
      clearInterval(biliLoginPollTimer)
      biliLoginPollTimer = null
    }
    if (biliLoginTimeout) {
      clearTimeout(biliLoginTimeout)
      biliLoginTimeout = null
    }
  }

  /** 登录成功收尾：停止轮询、关窗、更新表单并提示。 */
  async function settleBiliLoginSuccess() {
    biliLoginSettled = true
    stopBiliLoginPolling()
    biliLoginBusy.value = false
    await closeBilibiliLoginWindow(BILI_LOGIN_WINDOW_LABEL)
    onLoginSuccess()
    showToast(t('settings.bilibili_login_success'))
  }

  /** 清除已保存的 B 站 Cookie（SESSDATA）：过期后回退匿名模式的唯一入口（F2）。
   *  命令层 `set_credential('bilibili_cookie', '')` 本就支持空值清除，但此前没有任何 UI 触发点。 */
  async function handleClearBilibiliCookie() {
    track('settings.bili_clear')
    try {
      await setCredential('bilibili_cookie', '')
      onCookieCleared()
      showToast(t('settings.bilibili_cookie_cleared'))
    } catch (e: unknown) {
      showToast(t('settings.save_failed') + (e instanceof Error ? e.message : String(e)))
    }
  }

  /** 等待窗口创建结果：created 到达 resolve、error 到达 reject（避免 error 先触发时 await 永久挂起）。 */
  function waitBiliWindowCreated(win: WebviewWindow): Promise<void> {
    return new Promise((resolve, reject) => {
      let settled = false
      win.once('tauri://error', () => {
        if (!settled) { settled = true; reject(new Error('bilibili login window create failed')) }
      })
      win.once('tauri://created', () => {
        if (!settled) { settled = true; resolve() }
      })
    })
  }

  /** 启动登录态轮询（每 2 秒检测一次，直到登录成功或窗口关闭）。 */
  function startBiliLoginPolling() {
    stopBiliLoginPolling()
    biliLoginPollTimer = setInterval(async () => {
      if (biliLoginSettled) return
      try {
        const ok = await readBilibiliLoginCookie(BILI_LOGIN_WINDOW_LABEL)
        if (ok) await settleBiliLoginSuccess()
      } catch (e: unknown) {
        const key = e instanceof InvokeI18nError ? e.key : null
        // 窗口已关闭 → 用户手动放弃，停止轮询（按原始错误 key 判断，不依赖翻译文案）
        if (key === 'err.bili_login_window_missing') {
          biliLoginSettled = true
          stopBiliLoginPolling()
          biliLoginBusy.value = false
        } else if (key === 'err.bili_login_not_logged_in') {
          // 未登录：继续轮询等待扫码完成
        } else {
          // 持续性错误（cookie 读取失败/网络失败等）：停止轮询并提示，
          // 避免进入 60 秒死循环（期间按钮锁死且无法提前解除）
          biliLoginSettled = true
          stopBiliLoginPolling()
          biliLoginBusy.value = false
          showToast(t('settings.bilibili_login_failed') + (e instanceof Error ? e.message : String(e)))
        }
      }
    }, 2000)
  }

  /** 创建（或复用已存在的）登录窗口并轮询检测登录态；成功后自动保存 SESSDATA 并关窗。 */
  async function handleBilibiliLogin() {
    if (biliLoginBusy.value) return
    track('settings.bili_login')
    const attempt = ++biliLoginAttempt
    biliLoginBusy.value = true
    biliLoginSettled = false
    stopBiliLoginPolling()
    try {
      // 先探测已有窗口（如上次轮询已因 60 秒超时停止、但窗口仍保留的场景）：
      // 窗口存在则直接恢复轮询，避免同 label 重复创建失败导致无法重试
      let needCreate = true
      try {
        await readBilibiliLoginCookie(BILI_LOGIN_WINDOW_LABEL)
        // 窗口存在且已登录：cookie 已由命令加密入库，直接收尾
        if (attempt !== biliLoginAttempt) return
        await settleBiliLoginSuccess()
        return
      } catch (e: unknown) {
        if (attempt !== biliLoginAttempt) return
        const key = e instanceof InvokeI18nError ? e.key : null
        // 窗口已不存在 → 需要创建；未登录 → 窗口存在，恢复轮询即可；
        // 其它持续性错误（cookie 读取失败等）→ 提示并停止，不无限重试
        if (key !== null && key !== 'err.bili_login_window_missing' && key !== 'err.bili_login_not_logged_in') {
          biliLoginBusy.value = false
          showToast(t('settings.bilibili_login_failed') + (e instanceof Error ? e.message : String(e)))
          return
        }
        needCreate = key === 'err.bili_login_window_missing'
      }
      if (needCreate) {
        const win = new WebviewWindow(BILI_LOGIN_WINDOW_LABEL, {
          title: t('settings.bilibili_login_title'),
          url: 'https://passport.bilibili.com/login',
          width: 460,
          height: 640,
          center: true,
          resizable: false,
        })
        try {
          await waitBiliWindowCreated(win)
        } catch {
          if (attempt !== biliLoginAttempt) return
          biliLoginBusy.value = false
          showToast(t('settings.bilibili_login_window_failed'))
          return
        }
      }
      startBiliLoginPolling()
      // 超时保护：60 秒未登录则停止轮询（窗口保留；再次点击会恢复轮询，不会重复建窗）。
      // 句柄被保存并在收尾/卸载/超时自身处清理，避免定时器跨挂载存活、多次尝试累积（F3）。
      biliLoginTimeout = setTimeout(() => {
        // 单调代次令牌：仅当仍是本次尝试且未成功收尾时才停止，
        // 旧流程残留的闭包（已发起新登录/已收尾）不再误改状态
        if (attempt === biliLoginAttempt && !biliLoginSettled) {
          stopBiliLoginPolling()
          biliLoginBusy.value = false
        }
      }, 60000)
    } catch (e: unknown) {
      if (attempt !== biliLoginAttempt) return
      stopBiliLoginPolling()
      biliLoginBusy.value = false
      showToast(t('settings.bilibili_login_window_failed') + (e instanceof Error ? e.message : String(e)))
    }
  }

  onUnmounted(stopBiliLoginPolling)

  return { biliLoginBusy, handleBilibiliLogin, handleClearBilibiliCookie }
}
