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
