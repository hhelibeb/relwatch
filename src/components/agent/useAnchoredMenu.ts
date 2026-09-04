// ── 锚点 fixed 定位菜单三件套（Teleport 浮层通用件）──
// 收敛 AgentWorkspace 两处同构实现：会话 ⋯ 菜单（148，右对齐锚点右缘）与
// pi 状态菜单（216，左对齐锚点左缘）——锚元素 rect → 视口钳制 → fixed 定位。
// 定位语义与既有实现逐行对齐，不"顺手改进"；唯一的新增行为是 §2.3 B1：
// 内部统一 registerOverlayActive() + document 级 Esc 监听关闭菜单。
//
// B1 落地要点（防误触最小化到托盘）：
// - Esc 监听必须挂 document 级 keydown（打开时注册、关闭/卸载时注销）。菜单
//   Teleport 到 body、焦点通常留在触发按钮上，挂在菜单元素上的 @keydown 永远
//   不会触发；只注册覆盖层而不挂监听，结果是「Esc 无反应」而非「Esc 关菜单」
//   （useEscapeToTray 命中覆盖层后只是让路 return，菜单自身不关）。
// - 时序无冲突：useEscapeToTray 是 document 捕获期监听，hasActiveOverlay() 命中
//   后直接 return 不最小化；本监听（冒泡期）随后关菜单。
// - Esc 关菜单后焦点回归触发按钮（对齐 useDropdown 的焦点管理习惯）。
import { computed, onUnmounted, ref, watch, type Ref } from 'vue'
import { registerOverlayActive } from '../../composables/contextMenuBus'

export interface AnchoredMenuOptions {
  /** 菜单宽度：与 .agent-ws-menu-* 的 min-width 保持一致（148 / 216） */
  width: number
  /** 'right' = 菜单右对齐锚点右缘；'left' = 菜单左对齐锚点左缘 */
  align: 'left' | 'right'
  /** 视口左右安全边距（现有两处实现均为 8） */
  margin?: number
  /** 菜单打开状态（调用方持有：rpcMenuOpen / computed(openMenuKey !== null)） */
  isOpen: Ref<boolean>
  /** Esc 关闭菜单时调用（把 open 状态置回关闭值） */
  onClose: () => void
}

export function useAnchoredMenu(options: AnchoredMenuOptions) {
  const { width, align, margin = 8, isOpen, onClose } = options

  const pos = ref<{ x: number; y: number }>({ x: 0, y: 0 })
  const style = computed(() => ({ left: pos.value.x + 'px', top: pos.value.y + 'px' }))

  // 最近一次定位的锚元素：Esc 关闭后焦点回归触发按钮
  let anchorEl: HTMLElement | null = null

  /** 以锚元素定位菜单。rect 取不到时保持原位置（与既有实现的 if (rect) 分支一致）。 */
  function place(anchor: HTMLElement | null) {
    anchorEl = anchor
    const rect = anchor?.getBoundingClientRect()
    if (!rect) return
    const x =
      align === 'right'
        ? Math.max(margin, Math.min(rect.right - width, window.innerWidth - width))
        : Math.max(margin, Math.min(rect.left, window.innerWidth - width))
    pos.value = { x, y: rect.bottom + 4 }
  }

  // 打开期间向覆盖层总线注册：Esc 应优先关菜单而非最小化到托盘（§2.3 B1）
  const unregisterOverlay = registerOverlayActive(() => isOpen.value)

  function handleDocumentKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !isOpen.value) return
    e.preventDefault()
    onClose()
    anchorEl?.focus()
  }

  watch(isOpen, (open) => {
    if (open) document.addEventListener('keydown', handleDocumentKeydown)
    else document.removeEventListener('keydown', handleDocumentKeydown)
  })

  onUnmounted(() => {
    document.removeEventListener('keydown', handleDocumentKeydown)
    unregisterOverlay()
  })

  // pos 供测试直接断言定位坐标（style 是其字符串化视图）
  return { pos, style, place }
}
