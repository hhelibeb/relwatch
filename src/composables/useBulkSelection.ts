import { computed, onMounted, onUnmounted, ref, watch, type ComputedRef, type Ref } from 'vue'
import { registerOverlayActive } from './contextMenuBus'

export interface UseBulkSelectionOptions<T> {
  visibleIds: ComputedRef<readonly T[]>
  canSelect?: ComputedRef<boolean>
  active?: Ref<boolean> | ComputedRef<boolean>
  escToExit?: boolean
  canHandleEscape?: () => boolean
  onEscapeBeforeExit?: () => boolean
  onExit?: () => void
}

export function useBulkSelection<T>(options: UseBulkSelectionOptions<T>) {
  const selectionMode = ref(false)
  const selectedIds = ref<Set<T>>(new Set()) as Ref<Set<T>>
  const lastToggledId = ref<T | null>(null) as Ref<T | null>

  const selectedCount = computed(() => selectedIds.value.size)

  const allVisibleSelected = computed(() =>
    options.visibleIds.value.length > 0
    && options.visibleIds.value.every(id => selectedIds.value.has(id)),
  )

  watch(options.visibleIds, (ids) => {
    if (selectedIds.value.size === 0) return
    const valid = new Set(ids)
    let pruned = false
    const next = new Set<T>()
    for (const id of selectedIds.value) {
      if (valid.has(id)) next.add(id)
      else pruned = true
    }
    if (pruned) selectedIds.value = next
    if (lastToggledId.value !== null && !valid.has(lastToggledId.value)) {
      lastToggledId.value = null
    }
  })

  if (options.canSelect) {
    watch(options.canSelect, (ok) => {
      if (!ok && selectionMode.value) exitSelectionMode()
    })
  }

  function exitSelectionMode() {
    selectionMode.value = false
    selectedIds.value = new Set()
    lastToggledId.value = null
    options.onExit?.()
  }

  function toggleSelectionMode(): boolean {
    if (options.canSelect && !options.canSelect.value) return false
    if (selectionMode.value) {
      exitSelectionMode()
    } else {
      selectionMode.value = true
    }
    return true
  }

  function toggle(id: T, shiftKey = false) {
    const next = new Set(selectedIds.value)
    const shouldSelect = !next.has(id)
    if (shiftKey && lastToggledId.value !== null) {
      const orderedIds = options.visibleIds.value
      const from = orderedIds.indexOf(lastToggledId.value)
      const to = orderedIds.indexOf(id)
      if (from >= 0 && to >= 0) {
        const [start, end] = from <= to ? [from, to] : [to, from]
        for (let i = start; i <= end; i++) {
          if (shouldSelect) next.add(orderedIds[i])
          else next.delete(orderedIds[i])
        }
        selectedIds.value = next
        lastToggledId.value = id
        return
      }
    }
    if (shouldSelect) next.add(id)
    else next.delete(id)
    selectedIds.value = next
    lastToggledId.value = id
  }

  function selectAllVisible() {
    selectedIds.value = new Set(options.visibleIds.value)
  }

  function clearSelected() {
    selectedIds.value = new Set()
    lastToggledId.value = null
  }

  function isActive(): boolean {
    return options.active ? options.active.value : true
  }

  const escEnabled = options.escToExit ?? true

  function onKeydown(e: KeyboardEvent) {
    if (!escEnabled || !isActive() || !selectionMode.value) return
    if (e.defaultPrevented || e.isComposing || e.ctrlKey || e.altKey || e.metaKey) return
    if (e.key !== 'Escape') return
    if (options.canHandleEscape && !options.canHandleEscape()) return
    e.preventDefault()
    if (options.onEscapeBeforeExit?.()) return
    exitSelectionMode()
  }

  let unregisterOverlay: (() => void) | null = null

  onMounted(() => {
    unregisterOverlay = registerOverlayActive(() => isActive() && selectionMode.value)
    if (escEnabled) {
      window.addEventListener('keydown', onKeydown)
    }
  })

  onUnmounted(() => {
    if (escEnabled) {
      window.removeEventListener('keydown', onKeydown)
    }
    unregisterOverlay?.()
    unregisterOverlay = null
  })

  return {
    selectionMode,
    selectedIds,
    selectedCount,
    allVisibleSelected,
    toggleSelectionMode,
    exitSelectionMode,
    toggle,
    selectAllVisible,
    clearSelected,
  }
}
