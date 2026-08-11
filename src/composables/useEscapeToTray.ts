import { onMounted, onUnmounted, type Ref } from 'vue'
import { commands } from '../bindings'
import { hasActiveOverlay } from './contextMenuBus'

/**
 * Escape 逐层退出——最外层最小化到托盘
 *
 * 配合各组件的内置 Escape 处理器工作：
 * - 右键菜单 Escape → @close 关闭菜单
 * - 各下拉面板 Escape → 各自的 handle*Keydown 关闭面板
 * - 无覆盖层 + 已开启最小化到托盘 → 隐藏窗口到系统托盘
 *
 * 覆盖层判定走 contextMenuBus 的注册表（registerOverlayActive）：
 * 所有覆盖层（右键菜单/下拉/弹窗/面板）打开期间自行注册活跃回调，
 * 新增覆盖层时无需再改本文件——漏注册才会导致 Esc 误触最小化到托盘。
 */
export function useEscapeToTray(minimizeToTray: Ref<boolean>) {
  function handleKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return

    // 不拦截输入元素的 Escape（让原生行为处理，例如 blur 输入框）
    const tag = document.activeElement?.tagName
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
    if ((document.activeElement as HTMLElement | null)?.isContentEditable) return

    // 有覆盖层打开 → 让子组件的 Escape 处理器优先处理
    if (hasActiveOverlay()) return

    // 用户未开启最小化到托盘
    if (!minimizeToTray.value) return

    e.preventDefault()
    commands.hideToTray()
  }

  onMounted(() => document.addEventListener('keydown', handleKeydown, true))
  onUnmounted(() => document.removeEventListener('keydown', handleKeydown, true))
}
