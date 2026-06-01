/**
 * 全局右键菜单总线
 *
 * 用于协调所有独立的右键菜单实例，确保一次只显示一个菜单。
 * 每个拥有右键菜单的组件在打开自己菜单之前，先调用 closeAllContextMenus() 关闭全部。
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
