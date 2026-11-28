<script setup lang="ts">
import { t } from '../../i18n'

withDefaults(
  defineProps<{
    selectedCount: number
    selectedLabel: string
    selectAllLabel?: string
    clearLabel?: string
    busy?: boolean
  }>(),
  {
    selectAllLabel: undefined,
    clearLabel: undefined,
    busy: false,
  },
)

const emit = defineEmits<{
  selectAll: []
  clear: []
}>()
</script>

<template>
  <div class="bulk-bar" role="toolbar">
    <span class="bulk-count">{{ selectedLabel }}</span>
    <button class="btn-sm" :disabled="busy" @click="emit('selectAll')">
      <span class="bulk-btn-icon bulk-select-all-icon" aria-hidden="true">✓</span>
      <span>{{ selectAllLabel ?? t('source.bulk_select_all') }}</span>
    </button>
    <button class="btn-sm" :disabled="selectedCount === 0 || busy" @click="emit('clear')">
      <span class="bulk-btn-icon bulk-clear-icon" aria-hidden="true">✕</span>
      <span>{{ clearLabel ?? t('source.bulk_clear_selection') }}</span>
    </button>
    <slot />
  </div>
</template>

<style scoped>
.bulk-bar {
  display: flex;
  gap: 6px;
  align-items: center;
  flex-wrap: wrap;
  padding: 8px 0;
  background: var(--bg-subtle);
  border-bottom: 1px solid var(--border);
  font-size: var(--fs-meta);
}

.bulk-count {
  font-weight: 600;
  color: var(--text);
  margin-right: 4px;
}

.bulk-bar :deep(.btn-sm) {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  color: var(--text);
  font-size: var(--fs-meta);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--dur-hover) var(--ease-out);
}

.bulk-bar :deep(.bulk-btn-icon) {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
}

.bulk-select-all-icon,
.bulk-clear-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-meta);
  line-height: 1;
  font-weight: 700;
}

.bulk-bar :deep(.btn-sm:hover:not(:disabled)) {
  background: var(--bg-hover);
}

.bulk-bar :deep(.btn-sm:disabled) {
  opacity: 0.5;
  cursor: not-allowed;
}

.bulk-bar :deep(.btn-danger) {
  color: var(--danger);
}

.bulk-bar :deep(.btn-danger:hover:not(:disabled)) {
  background: var(--danger-soft-bg);
  border-color: var(--danger);
}
</style>
