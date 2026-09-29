import { onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { endTransientUiInvalidation, invalidateTransientUi, isTransientUiInvalidated } from './contextMenuBus'

/**
 * 窗口级瞬态 UI 守卫——「窗口走了再回来」不该把悬浮提示带回来
 *
 * 背景（实测 Edge/Chromium，WebView2 同内核）：
 *
 * 1. 关到托盘是 `window.hide()`（Win32 `SW_HIDE`），webview 与 DOM 都不销毁：
 *    组件状态、`document.activeElement` 全部原样保留。
 * 2. 隐藏瞬间 Chromium 只补 `blur` + `visibilitychange(hidden)`，
 *    **不补** `mouseleave`/`mouseout`（鼠标没有「离开」过，窗口直接不见了）。
 * 3. 重新显示时反过来会把「隐藏前获得焦点的元素」重放一次 `focus`，
 *    且不伴随任何鼠标事件、坐标也停在旧位置。
 *
 * 于是 `ReleaseItem` 的摘要提示（`@focus` 驱动、用元素 rect 定位）会在窗口重新出现时
 * 自己冒出来，而鼠标可能正停在托盘上。
 *
 * 本守卫做两件事：
 * - 窗口隐藏/失焦（DOM `blur`、`visibilitychange(hidden)`、Tauri 焦点变化三路信号）→
 *   `invalidateTransientUi()`：清空瞬态层并进入失效期。
 * - 等到「真实用户输入」才解除失效期：点击/按键/滚轮无条件解除；鼠标移动需坐标
 *   与失效前不同（坐标完全一致的移动视为窗口恢复时的重放事件，不放行）。
 *
 * 为什么不是「隐藏时清一次」就够了：重放的 `focus` 发生在窗口**显示之后**，
 * 只在隐藏时清理会被它重新建回来——失效期必须持续到用户真的动了手。
 */
export function useTransientUiGuard(options: { onInvalidate?: () => void } = {}) {
  const { onInvalidate } = options
  // 最后一次真实指针位置：作为「窗口恢复时的重放事件」识别基准
  let lastPointer: { x: number; y: number } | null = null
  // 失效期开始时的基准坐标（可能为 null：从未收到过指针事件）
  let stalePointer: { x: number; y: number } | null = null
  let unlistenFocus: (() => void) | null = null
  let mounted = false

  function invalidate() {
    stalePointer = lastPointer
    invalidateTransientUi()
    onInvalidate?.()
  }

  function release() {
    stalePointer = null
    endTransientUiInvalidation()
  }

  function onWindowBlur() {
    invalidate()
  }

  function onVisibilityChange() {
    if (document.hidden) invalidate()
  }

  // 点击/按键/滚轮必定来自用户：重放事件不会有这些
  function onUserInput() {
    if (isTransientUiInvalidated()) release()
  }

  function onPointerMove(e: MouseEvent | PointerEvent) {
    const x = e.clientX
    const y = e.clientY
    // 失效期内、坐标与失效前完全一致 → 视为重放，不放行也不更新基准
    if (stalePointer && stalePointer.x === x && stalePointer.y === y) return
    if (isTransientUiInvalidated()) release()
    lastPointer = { x, y }
  }

  const passiveCapture = { capture: true, passive: true } as const

  onMounted(async () => {
    mounted = true
    window.addEventListener('blur', onWindowBlur)
    window.addEventListener('mousedown', onUserInput, passiveCapture)
    window.addEventListener('pointerdown', onUserInput, passiveCapture)
    window.addEventListener('keydown', onUserInput, passiveCapture)
    window.addEventListener('wheel', onUserInput, passiveCapture)
    window.addEventListener('mousemove', onPointerMove, passiveCapture)
    window.addEventListener('pointermove', onPointerMove, passiveCapture)
    document.addEventListener('visibilitychange', onVisibilityChange)

    // Tauri 侧再挂一路：DOM blur 依赖 WM_KILLFOCUS 被转发到 webview，
    // 多一个信号源更稳（非 Tauri 环境静默跳过，单测/浏览器里不会走到）
    try {
      if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
        unlistenFocus = await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (mounted && !focused) invalidate()
        })
      }
    } catch {
      // 窗口已销毁 / 非 Tauri 运行时：忽略
    }
  })

  onUnmounted(() => {
    mounted = false
    window.removeEventListener('blur', onWindowBlur)
    window.removeEventListener('mousedown', onUserInput, passiveCapture)
    window.removeEventListener('pointerdown', onUserInput, passiveCapture)
    window.removeEventListener('keydown', onUserInput, passiveCapture)
    window.removeEventListener('wheel', onUserInput, passiveCapture)
    window.removeEventListener('mousemove', onPointerMove, passiveCapture)
    window.removeEventListener('pointermove', onPointerMove, passiveCapture)
    document.removeEventListener('visibilitychange', onVisibilityChange)
    unlistenFocus?.()
    unlistenFocus = null
    // 组件卸载后不应把失效期留在全局（否则下一个宿主里提示一直不工作）
    if (isTransientUiInvalidated()) release()
  })
}
