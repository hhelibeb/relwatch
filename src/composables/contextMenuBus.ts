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
 */
type Closer = () => void
const closers: Closer[] = []

export function registerCloser(closer: Closer) {
  closers.push(closer)
}

export function unregisterCloser(closer: Closer) {
  const idx = closers.indexOf(closer)
  if (idx !== -1) closers.splice(idx, 1)
}

/** 关闭所有已注册的右键菜单 */
export function closeAllContextMenus() {
  // 拷贝一份再遍历，避免迭代过程中被修改
  for (const closer of [...closers]) {
    closer()
  }
}

type OverlayActive = () => boolean
const overlayStates: OverlayActive[] = []

/**
 * 注册一个"覆盖层活跃判定"回调，返回注销函数。
 * 覆盖层打开期间回调应返回 true；关闭/卸载后必须注销，避免泄漏与误判。
 */
export function registerOverlayActive(isActive: OverlayActive): () => void {
  overlayStates.push(isActive)
  return () => {
    const idx = overlayStates.indexOf(isActive)
    if (idx !== -1) overlayStates.splice(idx, 1)
  }
}

/** 是否有任意覆盖层处于打开状态（供 useEscapeToTray 判定 Esc 是否应被覆盖层优先处理） */
export function hasActiveOverlay(): boolean {
  // 拷贝一份再遍历，避免回调中注销自身导致跳项
  return [...overlayStates].some((isActive) => isActive())
}

// ── 瞬态层失效期 ─────────────────────────────────────────────────────────────
//
// 为什么需要这一层：窗口隐藏到托盘走的是 Tauri `window.hide()` → Win32 `SW_HIDE`。
// 实测（Edge/Chromium，WebView2 同内核）：隐藏瞬间只补 `blur` + `visibilitychange`，
// **不补** `mouseleave`/`mouseout`；重新显示时反过来会把「隐藏前获得焦点的元素」
// 重放一次 `focus`（WINDOW focus + target focus），且不伴随任何鼠标事件。
// 于是 focus/hover 驱动的悬浮提示会"诈尸"：鼠标早已不在卡片上，提示却自己冒出来。
//
// 失效期语义：窗口刚回来、用户还没产生任何真实输入之前，瞬态提示一律静默。
// 置位时顺带 closeAllContextMenus()，把已经打开的菜单/提示就地清干净。
let transientInvalidated = false

/** 窗口隐藏/失焦：关闭全部瞬态层并进入失效期（由 useTransientUiGuard 调用） */
export function invalidateTransientUi() {
  transientInvalidated = true
  closeAllContextMenus()
}

/** 是否处于失效期——hover/focus 驱动的提示在此期间必须静默 */
export function isTransientUiInvalidated(): boolean {
  return transientInvalidated
}

/** 用户真实输入到达：解除失效期（由 useTransientUiGuard 调用） */
export function endTransientUiInvalidation() {
  transientInvalidated = false
}
