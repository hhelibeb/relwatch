import { onMounted, onUnmounted } from 'vue'
import { hasActiveOverlay } from './contextMenuBus'
import { isEditableTarget } from '../utils/dom'

export function useListKeyboardNav(opts: {
  isActive: () => boolean
  onMove: (delta: number) => void
  onOpenSelected: () => void
}) {
  function onKeydown(e: KeyboardEvent) {
    if (!opts.isActive() || e.isComposing || e.ctrlKey || e.altKey || e.metaKey) return
    if (hasActiveOverlay()) return
    if (isEditableTarget(e.target)) return
    if (e.key === 'j') {
      e.preventDefault()
      opts.onMove(1)
    } else if (e.key === 'k') {
      e.preventDefault()
      opts.onMove(-1)
    } else if (e.key === 'Enter') {
      if (e.repeat) return
      const target = e.target instanceof HTMLElement ? e.target : null
      if (target?.closest('button, a, [role="menu"], [role="menuitem"], [role="button"], [tabindex]:not([tabindex="-1"])')) return
      opts.onOpenSelected()
    }
  }

  onMounted(() => window.addEventListener('keydown', onKeydown))
  onUnmounted(() => window.removeEventListener('keydown', onKeydown))
}
