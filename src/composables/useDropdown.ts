import { watch, onUnmounted, type Ref } from 'vue'
import { registerOverlayActive } from './contextMenuBus'

export interface UseDropdownOptions<T> {
  /** 下拉组共享的打开状态：如 ref<'status'|'importance'|null>、ref<boolean> 或 ref<number|null> */
  openState: Ref<T>
  /** 该下拉组"全部关闭"时的取值（如 null / false） */
  closedKey: T
  /** 是否支持鼠标悬停展开（筛选类下拉用 true，点击式下拉用 false） */
  hoverOpen?: boolean
  /** 移出触发区后延迟关闭的毫秒数 */
  closeDelay?: number
}

/**
 * 下拉/菜单的通用交互逻辑，统一 SourceTab / ReleaseSearchBar / LogTab 三处实现：
 * - 点击触发开关（openedByClick 标记：点击打开的下拉不因 hover 移出自动关闭）
 * - 可选 hover 展开 + 移出延迟关闭（timer 在进入任一下拉区时清除）
 * - 触发按钮键盘：Enter/Space/ArrowDown 打开，Escape 关闭
 * - 下拉面板键盘：ArrowUp/Down 循环移动焦点，Escape 关闭
 * - 打开时自动聚焦面板内第一个可用选项（面板需带 role="menu" 或位于触发元素父级下）
 * - 打开期间向全局覆盖层总线注册活跃状态（供 useEscapeToTray 判定 Esc 拦截）
 * 共享 openState 即天然互斥：同组多个下拉（如状态/重要度）只有一个能打开。
 */
export function useDropdown<T>(options: UseDropdownOptions<T>) {
  const { openState, closedKey, hoverOpen = true, closeDelay = 120 } = options

  let hoverTimer: ReturnType<typeof setTimeout> | null = null
  let openedByClick = false

  // 打开期间向覆盖层总线注册：下拉打开时 Esc 应优先关闭下拉而非最小化到托盘
  const unregisterOverlay = registerOverlayActive(() => openState.value !== closedKey)

  watch(openState, () => {
    if (openState.value === closedKey) openedByClick = false
  })

  function clearHoverTimer() {
    if (hoverTimer) {
      clearTimeout(hoverTimer)
      hoverTimer = null
    }
  }

  /** 打开时聚焦面板内第一个可用选项；从触发元素就近定位，避免全局选择器误中其他实例 */
  function focusFirstItem(el: HTMLElement) {
    const panel = el.parentElement?.querySelector('[role="menu"]') as HTMLElement | null
    if (!panel) return
    const enabled = panel.querySelector('button:not(:disabled)') as HTMLButtonElement | null
    const any = panel.querySelector('button') as HTMLButtonElement | null
    ;(enabled ?? any)?.focus()
  }

  // openState 同步更新后，v-if 包裹的面板要到下一帧才渲染。
  // 聚焦必须等 DOM 更新后再执行；el 提前捕获，
  // 因为事件回调结束后 currentTarget 会变为 null。
  function deferFocus(el: HTMLElement) {
    const run = () => focusFirstItem(el)
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run)
    else setTimeout(run, 0)
  }

  function hoverEnter(key: T) {
    if (!hoverOpen) return
    clearHoverTimer()
    openState.value = key
  }

  function hoverLeave() {
    if (!hoverOpen) return
    if (openedByClick) return
    hoverTimer = setTimeout(() => {
      openState.value = closedKey
      hoverTimer = null
    }, closeDelay)
  }

  function toggle(e: MouseEvent, key: T) {
    const el = e.currentTarget as HTMLElement
    if (openState.value === key) {
      openState.value = closedKey
    } else {
      openState.value = key
      openedByClick = true
      deferFocus(el)
    }
  }

  function handleTriggerKeydown(e: KeyboardEvent, key: T) {
    const el = e.currentTarget as HTMLElement
    if (e.key === 'Escape') {
      if (openState.value === key) openState.value = closedKey
      return
    }
    if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
      e.preventDefault()
      if (openState.value !== key) {
        openState.value = key
        deferFocus(el)
      }
    }
  }

  function handleDropdownKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement
    if (!target || target.tagName !== 'BUTTON') return
    const dropdown = target.closest('[role="menu"]') as HTMLElement | null
    if (!dropdown) return
    const buttons = Array.from(dropdown.querySelectorAll('button')) as HTMLButtonElement[]
    const index = buttons.indexOf(target as HTMLButtonElement)
    if (index < 0) return
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      buttons[(index + 1) % buttons.length].focus()
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      buttons[(index - 1 + buttons.length) % buttons.length].focus()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      openState.value = closedKey
    }
  }

  function close() {
    openState.value = closedKey
  }

  onUnmounted(() => {
    clearHoverTimer()
    unregisterOverlay()
  })

  return { toggle, close, hoverEnter, hoverLeave, handleTriggerKeydown, handleDropdownKeydown }
}
