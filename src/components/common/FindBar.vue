<script setup lang="ts">
import { computed, type ComponentPublicInstance } from 'vue'

const props = withDefaults(defineProps<{
  modelValue: string
  matchCount: number
  currentIndex: number
  placeholder: string
  prevLabel: string
  nextLabel: string
  closeLabel: string
  classPrefix?: string
  compact?: boolean
  setInputEl?: (el: HTMLInputElement | null) => void
}>(), {
  classPrefix: '',
  compact: false,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  prev: []
  next: []
  close: []
  inputKeydown: [e: KeyboardEvent]
}>()

const countText = computed(() => {
  if (!props.modelValue.trim()) return ''
  return `${props.currentIndex >= 0 ? props.currentIndex + 1 : 0} / ${props.matchCount}`
})

function bindInputEl(el: Element | ComponentPublicInstance | null) {
  props.setInputEl?.(el instanceof HTMLInputElement ? el : null)
}

function onInput(e: Event) {
  emit('update:modelValue', (e.target as HTMLInputElement).value)
}
</script>

<template>
  <div class="find-bar" :class="[classPrefix, { 'find-bar-compact': compact }]">
    <input
      :ref="bindInputEl"
      :value="modelValue"
      type="text"
      class="find-bar-input"
      :class="classPrefix ? `${classPrefix}-input` : undefined"
      :placeholder="placeholder"
      :aria-label="placeholder"
      @input="onInput"
      @keydown="emit('inputKeydown', $event)"
    />
    <span
      v-if="countText"
      class="find-bar-count"
      :class="[classPrefix ? `${classPrefix}-count` : undefined, { 'no-match': matchCount === 0 }]"
    >{{ countText }}</span>
    <button
      type="button"
      class="find-bar-btn"
      :class="classPrefix ? `${classPrefix}-btn` : undefined"
      :disabled="matchCount === 0"
      :title="`${prevLabel} (Shift+Enter)`"
      :aria-label="prevLabel"
      @click="emit('prev')"
    >
      <svg viewBox="0 0 16 16"><polyline points="4 10 8 6 12 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
    </button>
    <button
      type="button"
      class="find-bar-btn"
      :class="classPrefix ? `${classPrefix}-btn` : undefined"
      :disabled="matchCount === 0"
      :title="`${nextLabel} (Enter)`"
      :aria-label="nextLabel"
      @click="emit('next')"
    >
      <svg viewBox="0 0 16 16"><polyline points="4 6 8 10 12 6" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
    </button>
    <button
      type="button"
      class="find-bar-btn"
      :class="classPrefix ? `${classPrefix}-btn` : undefined"
      :title="`${closeLabel} (Esc)`"
      :aria-label="closeLabel"
      @click="emit('close')"
    >
      <svg viewBox="0 0 16 16"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/></svg>
    </button>
  </div>
</template>

<style scoped>
.find-bar {
  display: flex;
  align-items: center;
  gap: 2px;
  margin: 8px 16px 0;
  padding: 4px 6px 4px 10px;
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  transition: border-color var(--dur-state) var(--ease-out), background var(--dur-state) var(--ease-out);
}

.find-bar-compact {
  margin: 8px 10px 0;
  padding: 3px 4px 3px 8px;
}

.find-bar:focus-within {
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
  background: var(--input-bg);
}

.find-bar-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: var(--fs-body);
  line-height: 1.4;
}

.find-bar-input::placeholder {
  color: var(--text-muted);
}

.find-bar-count {
  padding: 0 6px;
  font-family: var(--mono-font);
  font-size: var(--fs-meta);
  color: var(--text-muted);
  white-space: nowrap;
}

.find-bar-compact .find-bar-count {
  padding: 0 4px;
}

.find-bar-count.no-match {
  color: var(--danger);
}

.find-bar-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out);
}

.find-bar-compact .find-bar-btn {
  width: 22px;
  height: 22px;
}

.find-bar-btn:hover:not(:disabled) {
  background: var(--hover-surface-nested);
  color: var(--text);
}

.find-bar-btn:disabled {
  color: var(--border-strong);
  cursor: default;
}

.find-bar-btn svg {
  width: 14px;
  height: 14px;
}

.find-bar-compact .find-bar-btn svg {
  width: 13px;
  height: 13px;
}
</style>
