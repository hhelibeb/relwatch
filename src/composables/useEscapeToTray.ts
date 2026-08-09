import { onMounted, onUnmounted, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

/**
 * 覆盖层 CSS 选择器列表——任意一个在 DOM 中出现，Escape 就不应冒泡到 app 级。
 *
 * 这些元素用 `v-if` 控制显隐，打开时存在于 DOM、关闭时被移除，
 * 所以 `document.querySelector` 可以准确反映当前是否有覆盖层打开。
 */
const OVERLAY_SELECTORS = [
  '.context-menu',           // 右键菜单
  '.filter-dropdown',        // 日志/发布过滤下拉
  '.theme-select-dropdown',  // 设置页语言/主题选择器
  '.sort-dropdown',          // 监控源排序下拉
  '.dropdown-more-panel',    // 监控源更多操作面板
  '.release-detail-overlay', // 版本详情弹窗（漏掉会导致弹窗内 Esc 误触最小化到托盘）
]

function hasVisibleOverlay(): boolean {
  return document.querySelector(OVERLAY_SELECTORS.join(',')) !== null
}

/**
 * Escape 逐层退出——最外层最小化到托盘
 *
 * 配合各组件的内置 Escape 处理器工作：
 * - 右键菜单 Escape → @close 关闭菜单
 * - 各下拉面板 Escape → 各自的 handle*Keydown 关闭面板
 * - 无覆盖层 + 已开启最小化到托盘 → 隐藏窗口到系统托盘
 */
export function useEscapeToTray(minimizeToTray: Ref<boolean>) {
  function handleKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return

    // 不拦截输入元素的 Escape（让原生行为处理，例如 blur 输入框）
    const tag = document.activeElement?.tagName
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
    if ((document.activeElement as HTMLElement | null)?.isContentEditable) return

    // 有覆盖层打开 → 让子组件的 Escape 处理器优先处理
    if (hasVisibleOverlay()) return

    // 用户未开启最小化到托盘
    if (!minimizeToTray.value) return

    e.preventDefault()
    invoke('hide_to_tray')
  }

  onMounted(() => document.addEventListener('keydown', handleKeydown, true))
  onUnmounted(() => document.removeEventListener('keydown', handleKeydown, true))
}
