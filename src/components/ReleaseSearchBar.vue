<script setup lang="ts">
import { ref, computed } from 'vue'
import { t } from '../i18n'

const props = defineProps<{
  modelValue: string
  statusFilter: string
  importanceFilter: string
  viewMode: string
  showSearch?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: string]
  'update:importanceFilter': [value: string]
  'update:viewMode': [value: string]
  searchEnter: []
}>()

// ========== 筛选下拉状态 ==========
const openFilter = ref<'status' | 'importance' | null>(null)
let hoverFilterTimer: ReturnType<typeof setTimeout> | null = null

function hoverFilterEnter() {
  if (hoverFilterTimer) {
    clearTimeout(hoverFilterTimer)
    hoverFilterTimer = null
  }
}

function hoverFilterLeave() {
  hoverFilterTimer = setTimeout(() => {
    openFilter.value = null
  }, 120)
}

function toggleFilter(filter: 'status' | 'importance') {
  if (openFilter.value === filter) {
    openFilter.value = null
  } else {
    openFilter.value = filter
    focusFirstDropdownOption(filter)
  }
}

function handleFilterKeydown(e: KeyboardEvent, filter: 'status' | 'importance') {
  if (e.key === 'Escape') {
    if (openFilter.value !== null) {
      openFilter.value = null
    }
    return
  }
  if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
    e.preventDefault()
    if (openFilter.value !== filter) {
      openFilter.value = filter
      focusFirstDropdownOption(filter)
    }
  }
}

function handleDropdownKeydown(e: KeyboardEvent) {
  const target = e.target as HTMLElement
  if (!target || target.tagName !== 'BUTTON') return
  const dropdown = target.closest('.filter-dropdown') as HTMLElement | null
  if (!dropdown) return
  const buttons = Array.from(dropdown.querySelectorAll('button')) as HTMLButtonElement[]
  const index = buttons.indexOf(target as HTMLButtonElement)
  if (index < 0) return
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    const next = (index + 1) % buttons.length
    buttons[next].focus()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    const prev = (index - 1 + buttons.length) % buttons.length
    buttons[prev].focus()
  } else if (e.key === 'Escape') {
    e.preventDefault()
    openFilter.value = null
  }
}

function focusFirstDropdownOption(filter: 'status' | 'importance') {
  requestAnimationFrame(() => {
    // status 是第 1 个 filter-field，importance 是第 3 个（中间有 divider）
    const selector = filter === 'status' ? '.filter-field:nth-child(1)' : '.filter-field:nth-child(3)'
    const dropdown = document.querySelector(selector + ' .filter-dropdown') as HTMLElement | null
    if (!dropdown) return
    const btn = dropdown.querySelector('button') as HTMLButtonElement | null
    if (btn) btn.focus()
  })
}

const importanceDisplayText = computed(() => {
  if (props.importanceFilter === '大') return '🔴 ' + t('release.importance_high')
  if (props.importanceFilter === '中') return '🟡 ' + t('release.importance_medium')
  if (props.importanceFilter === '小') return '🟢 ' + t('release.importance_low')
  return t('release.filter_all')
})

function onSearchEnter() {
  emit('searchEnter')
}
</script>

<template>
  <div class="log-search-row">
    <div v-if="showSearch !== false" class="input-clear-wrap">
      <input
        :value="modelValue"
        :placeholder="t('release.search')"
        class="search-input"
        @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
        @keydown.enter.prevent="onSearchEnter"
      />
      <button v-if="modelValue" type="button" class="input-clear-btn" :title="t('input.clear')" @click="emit('update:modelValue', '')">✕</button>
    </div>
    <div class="filter-group" @mouseleave="hoverFilterLeave()">
      <div class="filter-field" @mouseenter="openFilter = 'status'; hoverFilterEnter()">
        <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'status'" aria-haspopup="menu" @click="toggleFilter('status')" @keydown="handleFilterKeydown($event, 'status')">
          <span class="filter-label">{{ t('tab.status') }}</span>
          <span class="filter-value" :style="{ color: props.statusFilter === 'unread' ? 'var(--primary)' : props.statusFilter === 'read' ? 'var(--success)' : 'var(--text-muted)' }">{{ props.statusFilter === 'all' ? t('release.filter_all') : (props.statusFilter === 'unread' ? t('release.filter_unread') : t('release.filter_read')) }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'status'" class="filter-dropdown" role="menu" @mouseenter="hoverFilterEnter()" @mouseleave="hoverFilterLeave()" @keydown="handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'all'" :class="{ selected: props.statusFilter === 'all' }" @click="emit('update:statusFilter', 'all'); openFilter = null">{{ t('release.filter_all') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'unread'" :class="{ selected: props.statusFilter === 'unread' }" @click="emit('update:statusFilter', 'unread'); openFilter = null">{{ t('release.filter_unread') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'read'" :class="{ selected: props.statusFilter === 'read' }" @click="emit('update:statusFilter', 'read'); openFilter = null">{{ t('release.filter_read') }}</button>
        </div>
      </div>
      <div class="filter-divider"></div>
      <div class="filter-field" @mouseenter="openFilter = 'importance'; hoverFilterEnter()">
        <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'importance'" aria-haspopup="menu" @click="toggleFilter('importance')" @keydown="handleFilterKeydown($event, 'importance')">
          <span class="filter-label">{{ t('tab.importance') }}</span>
          <span class="filter-value" :style="{ color: props.importanceFilter !== 'all' ? 'var(--text)' : 'var(--text-muted)' }">{{ importanceDisplayText }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'importance'" class="filter-dropdown" role="menu" @mouseenter="hoverFilterEnter()" @mouseleave="hoverFilterLeave()" @keydown="handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === 'all'" :class="{ selected: props.importanceFilter === 'all' }" @click="emit('update:importanceFilter', 'all'); openFilter = null">{{ t('release.filter_all') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '大'" :class="{ selected: props.importanceFilter === '大' }" @click="emit('update:importanceFilter', '大'); openFilter = null">🔴 {{ t('release.importance_high') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '中'" :class="{ selected: props.importanceFilter === '中' }" @click="emit('update:importanceFilter', '中'); openFilter = null">🟡 {{ t('release.importance_medium') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '小'" :class="{ selected: props.importanceFilter === '小' }" @click="emit('update:importanceFilter', '小'); openFilter = null">🟢 {{ t('release.importance_low') }}</button>
        </div>
      </div>
    </div>
    <div class="view-tabs">
      <button :class="{ active: props.viewMode === 'simple' }" @click="emit('update:viewMode', 'simple')">
        <svg><use href="/icons.svg#list-icon"/></svg>
        {{ t('release.view_simple') }}
      </button>
      <button :class="{ active: props.viewMode === 'aggregated' }" @click="emit('update:viewMode', 'aggregated')">
        <svg><use href="/icons.svg#grid-icon"/></svg>
        {{ t('release.view_aggregated') }}
      </button>
      <button :class="{ active: props.viewMode === 'calendar' }" @click="emit('update:viewMode', 'calendar')">
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
  background: var(--bg);
  border-radius: var(--radius);
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
  color: #6b7280;
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.15s;
  white-space: nowrap;
}

.view-tabs button:hover {
  background: rgba(0,0,0,0.04);
}

.view-tabs button.active {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(0,0,0,0.08);
}

.view-tabs button svg {
  width: 14px;
  height: 14px;
}

/* 筛选下拉框（自定义 hover 展开） */
.filter-group {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: visible;
  flex-shrink: 0;
}

.filter-field {
  position: relative;
}

.filter-field:first-child .filter-trigger {
  border-radius: 5px 0 0 5px;
}

.filter-field:last-child .filter-trigger {
  border-radius: 0 5px 5px 0;
}

.filter-trigger {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 6px 8px;
  border: none;
  background: var(--surface);
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.12s;
  height: 100%;
}

.filter-trigger:hover {
  background: var(--bg);
}

.filter-divider {
  width: 1px;
  background: var(--border);
  align-self: stretch;
}

.filter-label {
  font-size: 10px;
  color: var(--text-muted);
}

.filter-arrow {
  width: 12px;
  height: 12px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.filter-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 50%;
  transform: translateX(-50%);
  z-index: 100;
  min-width: 100%;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0,0,0,0.12);
  padding: 4px;
  white-space: nowrap;
}

.filter-dropdown button {
  display: block;
  width: 100%;
  padding: 5px 14px;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  text-align: left;
  border-radius: 4px;
  transition: background 0.1s;
}

.filter-dropdown button:hover {
  background: var(--bg);
}

.filter-dropdown button.selected {
  font-weight: 600;
  color: var(--primary);
}

/* 状态下拉选项颜色（与版本列表 badge 一致） */
.filter-field:first-child .filter-dropdown button:nth-child(1),
.filter-field:first-child .filter-dropdown button:nth-child(1).selected {
  color: var(--text);
  font-weight: normal;
}

.filter-field:last-child .filter-dropdown button:nth-child(1),
.filter-field:last-child .filter-dropdown button:nth-child(1).selected {
  color: var(--text);
  font-weight: normal;
}

.filter-field:first-child .filter-dropdown button:nth-child(2) {
  color: var(--primary);
}
.filter-field:first-child .filter-dropdown button:nth-child(2):hover,
.filter-field:first-child .filter-dropdown button:nth-child(2).selected {
  background: #dbeafe;
}

.filter-field:first-child .filter-dropdown button:nth-child(3) {
  color: var(--success);
}
.filter-field:first-child .filter-dropdown button:nth-child(3):hover,
.filter-field:first-child .filter-dropdown button:nth-child(3).selected {
  background: #dcfce7;
}

:global([data-theme="dark"] .filter-field:first-child .filter-dropdown button:nth-child(2):hover),
:global([data-theme="dark"] .filter-field:first-child .filter-dropdown button:nth-child(2).selected) {
  background: rgba(59, 130, 246, 0.25);
}

:global([data-theme="dark"] .filter-field:first-child .filter-dropdown button:nth-child(3):hover),
:global([data-theme="dark"] .filter-field:first-child .filter-dropdown button:nth-child(3).selected) {
  background: rgba(74, 222, 128, 0.2);
}

:global([data-theme="dark"] .filter-dropdown) {
  box-shadow: 0 6px 20px rgba(0,0,0,0.4);
}


/* 搜索栏 + 视图切换同行 */
.log-search-row {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 12px;
  padding: 0;
  background: var(--bg);
  transition: top 0.15s ease;
}

.log-search-row .search-input {
  flex: 1;
  max-width: none;
}

.log-search-row .input-clear-wrap {
  flex: 1;
}

.log-search-row .input-clear-wrap .search-input {
  max-width: none;
}

/* 滚动后 sticky 元素显示分隔线 */
:global(.app-main.is-scrolled .log-search-row) {
  top: calc(-1 * var(--app-padding-y, 16px));
  border-radius: var(--radius);
  box-shadow: 0 0 0 1px var(--border), 0 2px 6px rgba(0,0,0,0.04);
}

:global([data-theme="dark"] .view-tabs button) {
  color: #94a3b8;
}
:global([data-theme="dark"] .view-tabs button:hover) {
  background: rgba(255,255,255,0.06);
}
:global([data-theme="dark"] .view-tabs button.active) {
  color: #ffffff;
  background: rgba(96, 165, 250, 0.2);
}
</style>
