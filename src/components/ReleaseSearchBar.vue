<script setup lang="ts">
import { ref, computed } from 'vue'
import { t } from '../i18n'
import { useDropdown } from '../composables/useDropdown'
import { track } from '../composables/useUsageTracking'

const props = withDefaults(defineProps<{
  modelValue: string
  statusFilter: string
  importanceFilter: string
  viewMode: string
  showSearch?: boolean
}>(), {
  showSearch: true,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: string]
  'update:importanceFilter': [value: string]
  'update:viewMode': [value: string]
  searchEnter: []
}>()

// ========== 筛选下拉状态 ==========
// 状态/重要度共享一个 open 状态：hover 打开互斥，点击打开的不会被 hover 移出自动关闭
const openFilter = ref<'status' | 'importance' | null>(null)
const filterDropdown = useDropdown({
  openState: openFilter,
  closedKey: null,
  hoverOpen: true,
  // 打开时聚焦下拉第一个选项；从触发元素就近定位，避免全局选择器误中其他实例
  onOpen: (_key, el) => {
    const dropdown = el.parentElement?.querySelector('.filter-dropdown') as HTMLElement | null
    dropdown?.querySelector('button')?.focus()
  },
})

const importanceDisplayText = computed(() => {
  if (props.importanceFilter === '大') return t('release.importance_high')
  if (props.importanceFilter === '中') return t('release.importance_medium')
  if (props.importanceFilter === '小') return t('release.importance_low')
  return t('release.filter_all')
})

// 重要度圆点样式类：用设计系统的语义色替代 emoji，保证跨平台渲染一致
const importanceDotClass = computed(() => {
  if (props.importanceFilter === '大') return 'importance-dot-high'
  if (props.importanceFilter === '中') return 'importance-dot-medium'
  if (props.importanceFilter === '小') return 'importance-dot-low'
  return ''
})

function onSearchEnter() {
  emit('searchEnter')
}

function selectStatusFilter(value: string) {
  emit('update:statusFilter', value)
  filterDropdown.close()
  track('release.filter_status')
}

function selectImportanceFilter(value: string) {
  emit('update:importanceFilter', value)
  filterDropdown.close()
  track('release.filter_importance')
}

function selectViewMode(value: string) {
  emit('update:viewMode', value)
  track('release.view_' + value)
}
</script>

<template>
  <div class="log-search-row">
    <div v-if="props.showSearch" class="input-clear-wrap">
      <input
        :value="modelValue"
        :placeholder="t('release.search')"
        class="search-input"
        @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
        @keydown.enter.prevent="onSearchEnter"
      />
      <button v-if="modelValue" type="button" class="input-clear-btn" :title="t('input.clear')" @click="emit('update:modelValue', '')">✕</button>
    </div>
    <div class="filter-group" @mouseleave="filterDropdown.hoverLeave()">
      <div class="filter-field" @mouseenter="filterDropdown.hoverEnter('status')">
        <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'status'" aria-haspopup="menu" @click="filterDropdown.toggle($event, 'status')" @keydown="filterDropdown.handleTriggerKeydown($event, 'status')">
          <span class="filter-label">{{ t('tab.status') }}</span>
          <span class="filter-value" :style="{ color: props.statusFilter === 'unread' ? 'var(--primary)' : props.statusFilter === 'read' ? 'var(--success)' : 'var(--text-muted)' }">{{ props.statusFilter === 'all' ? t('release.filter_all') : (props.statusFilter === 'unread' ? t('release.filter_unread') : t('release.filter_read')) }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'status'" class="filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('status')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'all'" :class="{ selected: props.statusFilter === 'all' }" @click="selectStatusFilter('all')">{{ t('release.filter_all') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'unread'" :class="{ selected: props.statusFilter === 'unread' }" @click="selectStatusFilter('unread')">{{ t('release.filter_unread') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'read'" :class="{ selected: props.statusFilter === 'read' }" @click="selectStatusFilter('read')">{{ t('release.filter_read') }}</button>
        </div>
      </div>
      <div class="filter-divider"></div>
      <div class="filter-field" @mouseenter="filterDropdown.hoverEnter('importance')">
        <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'importance'" aria-haspopup="menu" @click="filterDropdown.toggle($event, 'importance')" @keydown="filterDropdown.handleTriggerKeydown($event, 'importance')">
          <span class="filter-label">{{ t('tab.importance') }}</span>
          <span class="filter-value" :style="{ color: props.importanceFilter !== 'all' ? 'var(--text)' : 'var(--text-muted)' }"><span v-if="importanceDotClass" class="importance-dot" :class="importanceDotClass"></span>{{ importanceDisplayText }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'importance'" class="filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('importance')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === 'all'" :class="{ selected: props.importanceFilter === 'all' }" @click="selectImportanceFilter('all')">{{ t('release.filter_all') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '大'" :class="{ selected: props.importanceFilter === '大' }" @click="selectImportanceFilter('大')"><span class="importance-dot importance-dot-high"></span>{{ t('release.importance_high') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '中'" :class="{ selected: props.importanceFilter === '中' }" @click="selectImportanceFilter('中')"><span class="importance-dot importance-dot-medium"></span>{{ t('release.importance_medium') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '小'" :class="{ selected: props.importanceFilter === '小' }" @click="selectImportanceFilter('小')"><span class="importance-dot importance-dot-low"></span>{{ t('release.importance_low') }}</button>
        </div>
      </div>
    </div>
    <div class="view-tabs">
      <button :class="{ active: props.viewMode === 'simple' }" @click="selectViewMode('simple')">
        <svg><use href="/icons.svg#list-icon"/></svg>
        {{ t('release.view_simple') }}
      </button>
      <button :class="{ active: props.viewMode === 'aggregated' }" @click="selectViewMode('aggregated')">
        <svg><use href="/icons.svg#grid-icon"/></svg>
        {{ t('release.view_aggregated') }}
      </button>
      <button :class="{ active: props.viewMode === 'calendar' }" @click="selectViewMode('calendar')">
        <svg><use href="/icons.svg#calendar-icon"/></svg>
        {{ t('release.view_calendar') }}
      </button>
    </div>
  </div>
</template>
<style scoped>
/* 视图切换按钮组 */
.view-tabs {
  display: flex;
  gap: 4px;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
  padding: 2px;
  flex-shrink: 0;
}

.view-tabs button {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 5px 12px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 12px;
  transition: background 0.15s ease, color 0.15s ease, box-shadow 0.15s ease;
  white-space: nowrap;
}

.view-tabs button:hover {
  color: var(--text);
}

.view-tabs button.active {
  background: var(--control-active);
  color: var(--text);
  font-weight: 600;
}

.view-tabs button svg {
  width: 14px;
  height: 14px;
}

/* 重要度圆点：与版本卡片徽章同源的语义色，替代 emoji */
.importance-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  margin-right: 6px;
  border-radius: 999px;
  flex-shrink: 0;
}

.importance-dot-high {
  background: var(--danger);
}

.importance-dot-medium {
  background: var(--warning);
}

.importance-dot-low {
  background: var(--success);
}
</style>
