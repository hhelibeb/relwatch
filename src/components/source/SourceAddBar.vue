<script setup lang="ts">
import { computed, ref } from 'vue'
import { getSourceTypeDef, parseSourceUrl } from '../../api/sources'
import { t } from '../../i18n'
import type { SortDirection, SourceSortField } from './useSourceSortFilter'

export type HeaderMode = 'add' | 'search'

const props = defineProps<{
  headerMode: HeaderMode
  urlInput: string
  sourceSearch: string
  sortedCount: number
  hasActiveSourceSearch: boolean
  trimmedSourceSearch: string
  addBusy: boolean
  subscribeVideos: boolean
  subscribeLive: boolean
  subscribePosts: boolean
  openSort: boolean
  sourceSortField: SourceSortField
  sourceSortDirection: SortDirection
  sortLabelText: string
  sortFieldOptions: readonly { value: SourceSortField; label: string }[]
  sortDirectionOptions: readonly { value: SortDirection; label: string }[]
  selectionMode: boolean
  onSortTriggerClick: (e: MouseEvent) => void
  onSortTriggerKeydown: (e: KeyboardEvent) => void
  onSortDropdownKeydown: (e: KeyboardEvent) => void
}>()

const emit = defineEmits<{
  'update:urlInput': [value: string]
  'update:sourceSearch': [value: string]
  'update:subscribeVideos': [value: boolean]
  'update:subscribeLive': [value: boolean]
  'update:subscribePosts': [value: boolean]
  add: []
  toggleHeaderMode: []
  toggleSelectionMode: []
  selectSortField: [field: SourceSortField]
  selectSortDirection: [dir: SortDirection]
}>()

const urlInputEl = ref<HTMLInputElement | null>(null)

const isYoutubeInput = computed(() => {
  if (props.headerMode !== 'add') return false
  const parsed = parseSourceUrl(props.urlInput)
  return parsed ? !!getSourceTypeDef(parsed.type)?.hasConfigInput : false
})

const modeToggleTitle = computed(() => {
  if (props.headerMode === 'add') {
    return props.hasActiveSourceSearch
      ? t('source.search_active_tip', props.trimmedSourceSearch)
      : t('source.switch_search')
  }
  return t('source.switch_add')
})

function focusAddInput() {
  urlInputEl.value?.focus()
}

defineExpose({ focusAddInput })
</script>

<template>
  <div class="source-header">
    <div class="input-clear-wrap">
      <input
        v-if="headerMode === 'add'"
        ref="urlInputEl"
        :value="urlInput"
        :placeholder="t('source.placeholder')"
        @input="emit('update:urlInput', ($event.target as HTMLInputElement).value)"
        @keyup.enter="emit('add')"
      />
      <input
        v-else
        :value="sourceSearch"
        :placeholder="t('source.search')"
        class="search-input"
        @input="emit('update:sourceSearch', ($event.target as HTMLInputElement).value)"
      />
      <span
        v-if="headerMode === 'search'"
        class="source-count"
        :class="{ 'has-clear': sourceSearch !== '' }"
        :title="t('source.search_result_count', String(sortedCount))"
      >({{ sortedCount }})</span>
      <button
        v-if="headerMode === 'add' && urlInput"
        type="button"
        class="input-clear-btn"
        :title="t('input.clear')"
        @click="emit('update:urlInput', '')"
      >✕</button>
      <button
        v-else-if="headerMode === 'search' && sourceSearch"
        type="button"
        class="input-clear-btn"
        :title="t('input.clear')"
        @click="emit('update:sourceSearch', '')"
      >✕</button>
    </div>
    <button v-if="headerMode === 'add'" class="btn-add-source" :disabled="addBusy || !urlInput" @click="emit('add')">
      {{ t('source.add') }}
    </button>
    <div class="sort-group">
      <button
        type="button"
        class="sort-trigger"
        :aria-expanded="openSort"
        aria-haspopup="menu"
        @click.stop="onSortTriggerClick"
        @keydown="onSortTriggerKeydown"
      >
        <span class="sort-direction-icon" aria-hidden="true">{{ sourceSortDirection === 'asc' ? '↑' : '↓' }}</span>
        <span>{{ sortLabelText }}</span>
        <svg class="sort-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon" /></svg>
      </button>
      <div v-if="openSort" class="dropdown-panel sort-dropdown" role="menu" @click.stop @keydown="onSortDropdownKeydown">
        <button
          v-for="opt in sortFieldOptions"
          :key="opt.value"
          type="button"
          role="menuitem"
          :aria-selected="sourceSortField === opt.value"
          :class="{ selected: sourceSortField === opt.value }"
          @click="emit('selectSortField', opt.value)"
        >{{ opt.label }}</button>
        <div class="sort-dropdown-divider"></div>
        <button
          v-for="opt in sortDirectionOptions"
          :key="opt.value"
          type="button"
          role="menuitem"
          :aria-selected="sourceSortDirection === opt.value"
          :class="{ selected: sourceSortDirection === opt.value }"
          @click="emit('selectSortDirection', opt.value)"
        >{{ opt.label }}</button>
      </div>
    </div>
    <button class="btn-select" @click="emit('toggleSelectionMode')">
      <svg class="btn-select-icon" width="13" height="13" aria-hidden="true">
        <use :href="selectionMode ? '/icons.svg#checkbox-checked-icon' : '/icons.svg#checkbox-icon'" />
      </svg>
      {{ selectionMode ? t('source.select_cancel') : t('source.select') }}
    </button>
    <button
      type="button"
      class="btn-mode-toggle"
      :class="{ 'has-active-search': headerMode === 'add' && hasActiveSourceSearch }"
      :title="modeToggleTitle"
      :aria-label="modeToggleTitle"
      @click="emit('toggleHeaderMode')"
    >
      <svg v-if="headerMode === 'add'" width="16" height="16"><use href="/icons.svg#search-icon" /></svg>
      <span v-else class="mode-toggle-plus" aria-hidden="true">+</span>
      <span v-if="headerMode === 'add' && hasActiveSourceSearch" class="mode-toggle-dot" aria-hidden="true"></span>
    </button>
  </div>
  <div v-if="headerMode === 'add' && isYoutubeInput" class="yt-subscribe-row">
    <span class="yt-subscribe-label">{{ t('source.subscribe_title') }}</span>
    <label class="yt-subscribe-option">
      <input
        type="checkbox"
        :checked="subscribeVideos"
        @change="emit('update:subscribeVideos', ($event.target as HTMLInputElement).checked)"
      />{{ t('source.subscribe_videos') }}
    </label>
    <label class="yt-subscribe-option">
      <input
        type="checkbox"
        :checked="subscribeLive"
        @change="emit('update:subscribeLive', ($event.target as HTMLInputElement).checked)"
      />{{ t('source.subscribe_live') }}
    </label>
    <label class="yt-subscribe-option yt-subscribe-disabled" :title="t('source.posts_unsupported')">
      <input type="checkbox" :checked="subscribePosts" disabled />{{ t('source.subscribe_posts') }}
    </label>
  </div>
</template>

<style scoped>
.source-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0;
  background: var(--bg);
}

.source-header .input-clear-wrap {
  flex: 1;
  min-width: 160px;
}

.source-header .input-clear-wrap input {
  flex: 1;
  padding: 8px 12px;
  padding-right: 34px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: var(--fs-body);
  outline: none;
  transition: border-color var(--dur-state) var(--ease-out), box-shadow var(--dur-state) var(--ease-out);
}

.source-header .input-clear-wrap input:focus {
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}

.source-header .input-clear-wrap .search-input {
  max-width: none;
  padding-right: 72px;
}

.source-count {
  position: absolute;
  right: 10px;
  top: 50%;
  transform: translateY(-50%);
  font-size: var(--fs-meta);
  color: var(--text-muted);
  white-space: nowrap;
  pointer-events: none;
  transition: right var(--dur-state) var(--ease-out);
}

.source-count.has-clear {
  right: 34px;
}

.btn-add-source {
  padding: 8px 14px;
  background: var(--ink);
  color: var(--on-ink);
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: var(--fs-body);
  font-weight: 500;
  white-space: nowrap;
  transition: background var(--dur-state) var(--ease-out), transform var(--dur-hover) var(--ease-out);
}

.btn-add-source:hover:not(:disabled) {
  background: var(--ink-hover);
}

.btn-add-source:active:not(:disabled) {
  transform: translateY(1px);
}

.btn-add-source:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-mode-toggle {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  color: var(--text-muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--dur-hover) var(--ease-out);
}

.btn-mode-toggle:hover {
  background: var(--hover-surface);
  border-color: var(--border-strong);
  color: var(--text);
}

.btn-mode-toggle.has-active-search {
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
  color: var(--primary);
}

.mode-toggle-dot {
  position: absolute;
  top: 5px;
  right: 5px;
  width: 7px;
  height: 7px;
  border: 2px solid var(--surface);
  border-radius: var(--radius-pill);
  background: var(--primary);
}

.btn-mode-toggle.has-active-search:hover .mode-toggle-dot {
  border-color: var(--bg);
}

.btn-mode-toggle svg {
  width: 16px;
  height: 16px;
}

.mode-toggle-plus {
  font-size: var(--fs-display);
  line-height: 1;
  font-weight: 500;
}

.sort-group {
  position: relative;
  flex-shrink: 0;
}

.sort-trigger {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  font-size: var(--fs-meta);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--dur-hover) var(--ease-out), border-color var(--dur-hover) var(--ease-out);
}

.sort-trigger:hover {
  background: var(--hover-surface);
  border-color: var(--border-strong);
}

.sort-direction-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 13px;
  height: 13px;
  color: var(--primary);
  font-size: var(--fs-body);
  line-height: 1;
  font-weight: 700;
}

.sort-arrow {
  color: var(--text-muted);
}

.sort-dropdown {
  top: calc(100% + 4px);
  right: 0;
  min-width: 140px;
  padding: 4px;
}

.sort-dropdown button {
  display: block;
  width: 100%;
  padding: 5px 14px;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: var(--fs-meta);
  cursor: pointer;
  text-align: left;
  border-radius: var(--radius-sm);
  transition: background var(--dur-hover) var(--ease-out);
}

.sort-dropdown button:hover {
  background: var(--hover-surface);
}

.sort-dropdown button.selected {
  font-weight: 600;
  color: var(--primary);
}

.sort-dropdown-divider {
  height: 1px;
  margin: 4px 2px;
  background: var(--border);
}

.btn-select {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  font-size: var(--fs-meta);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--dur-hover) var(--ease-out), border-color var(--dur-hover) var(--ease-out);
  flex-shrink: 0;
}

.btn-select:hover {
  background: var(--hover-surface);
  border-color: var(--border-strong);
}

.yt-subscribe-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 14px;
  padding: 8px 2px 0;
  font-size: var(--fs-meta);
  color: var(--text-muted);
}

.yt-subscribe-label {
  font-weight: 500;
  color: var(--text);
}

.yt-subscribe-option {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  color: var(--text);
  user-select: none;
}

.yt-subscribe-option input[type="checkbox"] {
  width: 14px;
  height: 14px;
  cursor: pointer;
  accent-color: var(--primary);
}

.yt-subscribe-disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.yt-subscribe-disabled input[type="checkbox"] {
  cursor: not-allowed;
}
</style>
