/**
 * 全局覆盖层/右键菜单协调总线
 *
 * 两套注册协议，都依赖"注册即生效、漏注册不报错"的自觉约定：
 *
 * 1. 右键菜单互斥（registerCloser/closeAllContextMenus）：
 *    每个拥有右键菜单的组件在打开自己菜单之前，先调用 closeAllContextMenus() 关闭全部。
 *    漏注册的菜单不会被互斥，也不会得到任何提示——新增菜单宿主时必须注册。
 *
 * 2. 覆盖层活跃判定（registerOverlayActive/hasActiveOverlay）：
 *    供 useEscapeToTray 判断"是否有覆盖层打开"（Esc 不应最小化到托盘）。
 *    所有覆盖层（右键菜单、下拉面板、弹窗、开发者面板等）在打开期间必须注册活跃回调；
 *    漏注册会导致覆盖层内按 Esc 误触最小化到托盘。useDropdown / usePreviewSelect /
 *    ContextMenu / ReleaseDetailModal / StatsDevPanel 已内置注册。
 *
 * 3. 瞬态层失效期（invalidateTransientUi/isTransientUiInvalidated/endTransientUiInvalidation）：
 *    窗口隐藏到托盘或失焦后，Chromium 不补 mouseleave/mouseout，却会在窗口重新显示时
 *    把「隐藏前获得焦点的元素」重放一次 focus；纯 UI 状态的悬浮提示因此会"诈尸"。
 *    由窗口级守卫（useTransientUiGuard）在隐藏/失焦时调用 invalidateTransientUi()，
 *    在真实用户输入时调用 endTransientUiInvalidation()；hover/focus 驱动的提示
 *    必须自行检查 isTransientUiInvalidated() 并在失效期内保持静默。
 *
 * 4. 页内查找接管（registerFindHandler/dispatchFind）：
 *    供 App.vue 的全局 Ctrl+F 按 LIFO 顺序询问最顶层可查找浮层（如 ReleaseDetailModal）
 *    是否消费，避免跨层 ref 透传。接管器可返回 false 主动让路——常驻挂载但当前
 *    并非焦点的面板（如 Agent 工作区侧栏）必须这样做，否则会挤掉版本页搜索。
 */
import { reactive } from 'vue'

type Closer = () => void
const closers: Closer[] = []

type OverlayActive = () => boolean
const overlayStates: OverlayActive[] = []

type FindHandler = () => boolean
const findHandlers: FindHandler[] = []

export const overlayRegistryState = reactive({
  overlayVersion: 0,
  transientInvalidated: false,
})

export function registerCloser(closer: Closer) {
  closers.push(closer)
}

export function unregisterCloser(closer: Closer) {
  const idx = closers.indexOf(closer)
  if (idx !== -1) closers.splice(idx, 1)
}

export function closeAllContextMenus() {
  for (const closer of [...closers]) {
    closer()
  }
}

export function registerOverlayActive(isActive: OverlayActive): () => void {
  overlayStates.push(isActive)
  overlayRegistryState.overlayVersion++
  return () => {
    const idx = overlayStates.indexOf(isActive)
    if (idx !== -1) {
      overlayStates.splice(idx, 1)
      overlayRegistryState.overlayVersion++
    }
  }
}

export function hasActiveOverlay(): boolean {
  return [...overlayStates].some((isActive) => isActive())
}

export function registerFindHandler(handler: FindHandler): () => void {
  findHandlers.push(handler)
  return () => {
    const idx = findHandlers.indexOf(handler)
    if (idx !== -1) findHandlers.splice(idx, 1)
  }
}

export function dispatchFind(): boolean {
  const snapshot = [...findHandlers]
  for (let i = snapshot.length - 1; i >= 0; i--) {
    if (snapshot[i]()) return true
  }
  return false
}

export function invalidateTransientUi() {
  overlayRegistryState.transientInvalidated = true
  closeAllContextMenus()
}

export function isTransientUiInvalidated(): boolean {
  return overlayRegistryState.transientInvalidated
}

export function endTransientUiInvalidation() {
  overlayRegistryState.transientInvalidated = false
}
