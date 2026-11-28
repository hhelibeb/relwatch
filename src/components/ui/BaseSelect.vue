<script setup lang="ts">
import { computed } from 'vue'
import { usePreviewSelect } from '../../composables/usePreviewSelect'

export interface SelectOption {
  value: string
  label: string
}

const props = withDefaults(
  defineProps<{
    modelValue: string
    options: readonly SelectOption[]
    label?: string
    disabled?: boolean
    previewOnHover?: boolean
  }>(),
  {
    label: '',
    disabled: false,
    previewOnHover: false,
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: string]
  select: [value: string]
  preview: [value: string]
  restore: []
}>()

const {
  dropdownOpen,
  previewValue,
  selectRef,
  toggle,
  select,
  handleKeydown,
  clearPreview,
} = usePreviewSelect({
  preview: (val) => emit('preview', val),
  restore: () => emit('restore'),
  onSelect: (val) => {
    emit('update:modelValue', val)
    emit('select', val)
  },
  enablePreview: () => props.previewOnHover,
})

const currentLabel = computed(() => {
  const activeVal = props.previewOnHover && previewValue.value !== null ? previewValue.value : props.modelValue
  return props.options.find((opt) => opt.value === activeVal)?.label ?? activeVal
})

function onOptionEnter(val: string) {
  if (!props.previewOnHover) return
  previewValue.value = val
  emit('preview', val)
}

function onOptionSelect(val: string) {
  select(val)
}

function onTriggerClick() {
  if (props.disabled) return
  toggle()
}

defineExpose({ clearPreview })
</script>

<template>
  <div ref="selectRef" class="base-select theme-select" @mouseleave="clearPreview">
    <button
      type="button"
      class="theme-select-trigger setting-input"
      :disabled="disabled"
      :aria-expanded="dropdownOpen"
      :aria-label="label || undefined"
      aria-haspopup="listbox"
      @click="onTriggerClick"
      @keydown="handleKeydown"
    >
      <span>{{ currentLabel }}</span>
      <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
        <path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3" />
      </svg>
    </button>
    <div
      v-if="dropdownOpen"
      class="dropdown-panel theme-select-dropdown"
      role="listbox"
      @keydown="handleKeydown"
    >
      <div
        v-for="opt in options"
        :key="opt.value"
        class="theme-select-option"
        :class="{ selected: modelValue === opt.value && !previewValue, previewed: previewValue === opt.value }"
        :data-value="opt.value"
        tabindex="-1"
        role="option"
        :aria-selected="modelValue === opt.value"
        @mouseenter="onOptionEnter(opt.value)"
        @click.stop="onOptionSelect(opt.value)"
      >
        {{ opt.label }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.theme-select {
  position: relative;
  width: 18ch;
}

.theme-select-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  cursor: pointer;
  text-align: left;
  font-size: var(--fs-body);
}

.theme-select-trigger:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.theme-select-arrow {
  flex-shrink: 0;
  margin-left: 4px;
  transition: transform var(--dur-overlay) var(--ease-out);
}

.theme-select-dropdown {
  top: calc(100% + 4px);
  left: 0;
  width: 100%;
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.theme-select-option {
  padding: 8px 12px;
  font-size: var(--fs-body);
  cursor: pointer;
  color: var(--text);
  transition: background var(--dur-hover) var(--ease-out);
}

.theme-select-option:hover,
.theme-select-option.previewed {
  background: var(--hover-surface);
  color: var(--text);
}

.theme-select-option.selected {
  font-weight: 600;
}
</style>
