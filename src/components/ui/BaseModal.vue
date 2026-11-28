<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { useOverlayEscape } from '../../composables/useOverlayEscape'

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ')

const props = withDefaults(
  defineProps<{
    overlayClass?: string
    modalClass?: string
    modalStyle?: Record<string, string | number>
    closeOnBackdrop?: boolean
    escapeCapture?: boolean
    escapeStopPropagation?: boolean
    escapePreventDefault?: boolean
    ariaLabel?: string
    autoFocus?: boolean
    trapFocus?: boolean
    restoreFocus?: boolean
    onEscape?: () => void
    onKeydown?: (e: KeyboardEvent) => void
  }>(),
  {
    overlayClass: '',
    modalClass: '',
    modalStyle: undefined,
    closeOnBackdrop: true,
    escapeCapture: false,
    escapeStopPropagation: false,
    escapePreventDefault: false,
    ariaLabel: undefined,
    autoFocus: true,
    trapFocus: true,
    restoreFocus: true,
    onEscape: undefined,
    onKeydown: undefined,
  },
)

const emit = defineEmits<{ close: [] }>()

const modalEl = ref<HTMLElement | null>(null)
let previousActiveElement: HTMLElement | null = null
let backdropPointerDown = false

function isElementVisible(el: HTMLElement): boolean {
  if (el.closest('[hidden]')) return false
  const style = window.getComputedStyle(el)
  return style.display !== 'none' && style.visibility !== 'hidden'
}

function getFocusableElements(): HTMLElement[] {
  const root = modalEl.value
  if (!root) return []
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (el) => el.tabIndex !== -1 && !el.hasAttribute('disabled') && isElementVisible(el),
  )
}

function handleTrapKeydown(e: KeyboardEvent) {
  if (e.key === 'Tab' && props.trapFocus) {
    const focusables = getFocusableElements()
    if (focusables.length === 0) {
      e.preventDefault()
      modalEl.value?.focus()
    } else {
      const first = focusables[0]
      const last = focusables[focusables.length - 1]
      const active = document.activeElement as HTMLElement | null
      if (e.shiftKey) {
        if (!active || active === first || !modalEl.value?.contains(active)) {
          e.preventDefault()
          last.focus()
        }
      } else if (!active || active === last || !modalEl.value?.contains(active)) {
        e.preventDefault()
        first.focus()
      }
    }
  }
  props.onKeydown?.(e)
}

useOverlayEscape({
  onEscape: () => (props.onEscape ? props.onEscape() : emit('close')),
  capture: props.escapeCapture,
  stopPropagation: props.escapeStopPropagation,
  preventDefault: props.escapePreventDefault,
  onKeydown: handleTrapKeydown,
})

function onBackdropPointerDown(e: PointerEvent) {
  backdropPointerDown = e.target === e.currentTarget
}

function onBackdropClick(e: MouseEvent) {
  const fromBackdrop = backdropPointerDown || e.detail === 0
  backdropPointerDown = false
  if (props.closeOnBackdrop && fromBackdrop) emit('close')
}

onMounted(() => {
  previousActiveElement = document.activeElement instanceof HTMLElement ? document.activeElement : null
  if (!props.autoFocus) return
  void nextTick(() => {
    const root = modalEl.value
    if (!root) return
    if (document.activeElement && root.contains(document.activeElement)) return
    const focusables = getFocusableElements()
    if (focusables.length > 0) {
      focusables[0].focus()
    } else {
      root.focus()
    }
  })
})

onUnmounted(() => {
  const active = document.activeElement
  const focusStillInsideOrBody = !active || active === document.body || modalEl.value?.contains(active)
  if (props.restoreFocus && focusStillInsideOrBody && previousActiveElement && document.contains(previousActiveElement)) {
    previousActiveElement.focus()
  }
  previousActiveElement = null
})

defineExpose({ modalEl })
</script>

<template>
  <Teleport to="body">
    <div
      class="base-modal-overlay"
      :class="overlayClass"
      @pointerdown.self="onBackdropPointerDown"
      @click.self="onBackdropClick"
    >
      <div
        ref="modalEl"
        class="base-modal"
        :class="modalClass"
        :style="modalStyle"
        role="dialog"
        aria-modal="true"
        :aria-label="ariaLabel"
        tabindex="-1"
      >
        <slot />
      </div>
      <slot name="overlay-extra" />
    </div>
  </Teleport>
</template>

<style scoped>
.base-modal-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  padding: 24px;
}

.base-modal {
  display: flex;
  flex-direction: column;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  outline: none;
}
</style>
