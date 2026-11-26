import { onMounted, onUnmounted } from 'vue'
import { registerOverlayActive } from './contextMenuBus'

export interface UseOverlayEscapeOptions {
  onEscape: (e: KeyboardEvent) => void
  active?: () => boolean
  capture?: boolean
  stopPropagation?: boolean
  preventDefault?: boolean
  onKeydown?: (e: KeyboardEvent) => void
}

export function useOverlayEscape(options: UseOverlayEscapeOptions) {
  const capture = options.capture ?? false
  const isActive = options.active ?? (() => true)

  function handleKeydown(e: KeyboardEvent) {
    if (!isActive()) return
    if (e.defaultPrevented || e.isComposing) return
    if (e.key === 'Escape') {
      if (options.preventDefault) e.preventDefault()
      if (options.stopPropagation) e.stopPropagation()
      options.onEscape(e)
      return
    }
    options.onKeydown?.(e)
  }

  let unregisterOverlay: (() => void) | null = null

  onMounted(() => {
    window.addEventListener('keydown', handleKeydown, capture)
    unregisterOverlay = registerOverlayActive(isActive)
  })

  onUnmounted(() => {
    window.removeEventListener('keydown', handleKeydown, capture)
    unregisterOverlay?.()
    unregisterOverlay = null
  })
}
