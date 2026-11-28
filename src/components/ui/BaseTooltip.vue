<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { isTransientUiInvalidated } from '../../composables/contextMenuBus'

const props = withDefaults(
  defineProps<{
    visible: boolean
    text?: string
    x?: number
    y?: number
    anchorEl?: HTMLElement | null
    maxWidth?: number
    elevated?: boolean
    tooltipClass?: string
  }>(),
  {
    text: '',
    x: undefined,
    y: undefined,
    anchorEl: null,
    maxWidth: 420,
    elevated: false,
    tooltipClass: '',
  },
)

const tipEl = ref<HTMLElement | null>(null)
const pos = ref({ left: 0, top: 0 })

const shouldShow = computed(() => props.visible && !isTransientUiInvalidated())

function computePosition() {
  const margin = 12
  const vw = window.innerWidth
  const vh = window.innerHeight
  const el = tipEl.value
  const width = el?.offsetWidth || Math.min(props.maxWidth, vw - margin * 2)
  const height = el?.offsetHeight || 40

  let left = margin
  let top = margin

  if (props.anchorEl) {
    const rect = props.anchorEl.getBoundingClientRect()
    left = rect.left
    top = rect.bottom + 6
    if (top + height > vh - margin && rect.top - height - 6 >= margin) {
      top = rect.top - height - 6
    }
  } else if (props.x !== undefined && props.y !== undefined) {
    left = props.x + 12
    if (left + width > vw - margin && props.x - width - 12 >= margin) {
      left = props.x - width - 12
    }
    top = props.y + 12
    if (top + height > vh - margin && props.y - height - 12 >= margin) {
      top = props.y - height - 12
    }
  }

  left = Math.max(margin, Math.min(left, vw - width - margin))
  top = Math.max(margin, Math.min(top, vh - height - margin))
  pos.value = { left, top }
}

watch(
  () => [shouldShow.value, props.x, props.y, props.anchorEl, props.text],
  () => {
    if (!shouldShow.value) return
    computePosition()
    void nextTick(() => computePosition())
  },
  { immediate: true },
)

const tooltipStyle = computed(() => ({
  left: `${pos.value.left}px`,
  top: `${pos.value.top}px`,
  maxWidth: `${props.maxWidth}px`,
  zIndex: props.elevated ? 'var(--z-popover-modal)' : 'var(--z-popover)',
}))
</script>

<template>
  <Teleport to="body">
    <div
      v-if="shouldShow"
      ref="tipEl"
      class="base-tooltip"
      :class="tooltipClass"
      :style="tooltipStyle"
      role="tooltip"
    >
      <slot>{{ text }}</slot>
    </div>
  </Teleport>
</template>

<style>
:where(.base-tooltip) {
  position: fixed;
  padding: 6px 10px;
  background: var(--surface);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-md);
  font-size: var(--fs-meta);
  line-height: 1.5;
  pointer-events: none;
  overflow-wrap: anywhere;
}
</style>
