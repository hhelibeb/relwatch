<script setup lang="ts">
import { ref, computed } from 'vue'
import { t } from '../i18n'
import { useDropdown } from '../composables/useDropdown'
import { track } from '../composables/useUsageTracking'
import { getSourceTypeDef, sourceTypeDefs } from '../api/source-registry'
import type { ReleaseImportanceFilter, ReleaseSourceFilter, ReleaseStatusFilter, ViewMode } from './releaseTypes'

const props = withDefaults(defineProps<{
  modelValue: string
  statusFilter: ReleaseStatusFilter
  importanceFilter: ReleaseImportanceFilter
  sourceFilter: ReleaseSourceFilter
  viewMode: ViewMode
  showSearch?: boolean
}>(), {
  showSearch: true,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
  'update:importanceFilter': [value: ReleaseImportanceFilter]
  'update:sourceFilter': [value: ReleaseSourceFilter]
  'update:viewMode': [value: ViewMode]
  searchEnter: []
}>()

// ========== 筛选下拉状态 ==========
// 状态/重要度/来源/视图共享一个 open 状态：hover 打开互斥，点击打开的不会被 hover 移出自动关闭
const openFilter = ref<'status' | 'importance' | 'source' | 'view' | null>(null)
const filterDropdown = useDropdown({
  openState: openFilter,
  closedKey: null,
  hoverOpen: true,
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

// 来源筛选显示：全部无图标，选中类型显示类型徽标图标 + i18n 标题
const sourceDef = computed(() => {
  if (props.sourceFilter === 'all') return null
  return getSourceTypeDef(props.sourceFilter) ?? null
})
const sourceDisplayText = computed(() => {
  const def = sourceDef.value
  return def ? t(def.titleKey) : t('release.filter_all')
})

// 视图切换显示（折叠为下拉后沿用类型名文案）
const viewDisplayText = computed(() => {
  if (props.viewMode === 'aggregated') return t('release.view_aggregated')
  if (props.viewMode === 'calendar') return t('release.view_calendar')
  return t('release.view_simple')
})

// 视图图标：与旧按钮组同源（list/grid/calendar），触发按钮与下拉选项共用
const viewIconHref = computed(() => {
  if (props.viewMode === 'aggregated') return '/icons.svg#grid-icon'
  if (props.viewMode === 'calendar') return '/icons.svg#calendar-icon'
  return '/icons.svg#list-icon'
})

function onSearchEnter() {
  emit('searchEnter')
}

function selectStatusFilter(value: ReleaseStatusFilter) {
  emit('update:statusFilter', value)
  filterDropdown.close()
  track('release.filter_status')
}

function selectImportanceFilter(value: ReleaseImportanceFilter) {
  emit('update:importanceFilter', value)
  filterDropdown.close()
  track('release.filter_importance')
}

function selectSourceFilter(value: ReleaseSourceFilter) {
  emit('update:sourceFilter', value)
  filterDropdown.close()
  track('release.filter_source')
}

function selectViewMode(value: ViewMode) {
  emit('update:viewMode', value)
  filterDropdown.close()
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
        <div v-if="openFilter === 'status'" class="dropdown-panel filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('status')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
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
        <div v-if="openFilter === 'importance'" class="dropdown-panel filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('importance')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === 'all'" :class="{ selected: props.importanceFilter === 'all' }" @click="selectImportanceFilter('all')">{{ t('release.filter_all') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '大'" :class="{ selected: props.importanceFilter === '大' }" @click="selectImportanceFilter('大')"><span class="importance-dot importance-dot-high"></span>{{ t('release.importance_high') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '中'" :class="{ selected: props.importanceFilter === '中' }" @click="selectImportanceFilter('中')"><span class="importance-dot importance-dot-medium"></span>{{ t('release.importance_medium') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '小'" :class="{ selected: props.importanceFilter === '小' }" @click="selectImportanceFilter('小')"><span class="importance-dot importance-dot-low"></span>{{ t('release.importance_low') }}</button>
        </div>
      </div>
      <div class="filter-divider"></div>
      <div class="filter-field" @mouseenter="filterDropdown.hoverEnter('source')">
        <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'source'" aria-haspopup="menu" @click="filterDropdown.toggle($event, 'source')" @keydown="filterDropdown.handleTriggerKeydown($event, 'source')">
          <span class="filter-label">{{ t('tab.source') }}</span>
          <span class="filter-value" :style="{ color: props.sourceFilter !== 'all' ? 'var(--text)' : 'var(--text-muted)' }"><span v-if="sourceDef" class="filter-type-icon"><svg><use :href="sourceDef.icon"/></svg></span>{{ sourceDisplayText }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'source'" class="dropdown-panel filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('source')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.sourceFilter === 'all'" :class="{ selected: props.sourceFilter === 'all' }" @click="selectSourceFilter('all')">{{ t('release.filter_all') }}</button>
          <button v-for="def in sourceTypeDefs" :key="def.type" type="button" role="menuitem" :aria-selected="props.sourceFilter === def.type" :class="{ selected: props.sourceFilter === def.type }" @click="selectSourceFilter(def.type)"><span class="filter-type-icon"><svg><use :href="def.icon"/></svg></span>{{ t(def.titleKey) }}</button>
        </div>
      </div>
      <div class="filter-divider"></div>
      <div class="filter-field" @mouseenter="filterDropdown.hoverEnter('view')">
        <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'view'" aria-haspopup="menu" @click="filterDropdown.toggle($event, 'view')" @keydown="filterDropdown.handleTriggerKeydown($event, 'view')">
          <span class="filter-label">{{ t('tab.view') }}</span>
          <span class="filter-value" :style="{ color: props.viewMode !== 'simple' ? 'var(--text)' : 'var(--text-muted)' }"><span class="filter-type-icon"><svg><use :href="viewIconHref"/></svg></span>{{ viewDisplayText }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'view'" class="dropdown-panel filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('view')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.viewMode === 'simple'" :class="{ selected: props.viewMode === 'simple' }" @click="selectViewMode('simple')"><span class="filter-type-icon"><svg><use href="/icons.svg#list-icon"/></svg></span>{{ t('release.view_simple') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.viewMode === 'aggregated'" :class="{ selected: props.viewMode === 'aggregated' }" @click="selectViewMode('aggregated')"><span class="filter-type-icon"><svg><use href="/icons.svg#grid-icon"/></svg></span>{{ t('release.view_aggregated') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.viewMode === 'calendar'" :class="{ selected: props.viewMode === 'calendar' }" @click="selectViewMode('calendar')"><span class="filter-type-icon"><svg><use href="/icons.svg#calendar-icon"/></svg></span>{{ t('release.view_calendar') }}</button>
        </div>
      </div>
    </div>
  </div>
</template>
<style scoped>
/* 类型徽标图标（来源筛选触发按钮与下拉选项共用） */
.filter-type-icon {
  display: inline-flex;
  align-items: center;
  margin-right: 6px;
  flex-shrink: 0;
}

.filter-type-icon svg {
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
