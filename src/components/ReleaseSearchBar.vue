<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { t } from '../i18n'

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
const openFilter = ref<'status' | 'importance' | null>(null)
let hoverFilterTimer: ReturnType<typeof setTimeout> | null = null
// 标记当前下拉是否由点击打开：点击打开的不因 hover 离开自动关闭，
// 仅在点击选项/外部或 Escape 时关闭，避免用户移出后无法回头点击选项
let filterOpenedByClick = false

// 下拉关闭时重置 click 标记（覆盖选项点击、Escape、外部点击等所有关闭路径）
watch(openFilter, (v) => { if (v === null) filterOpenedByClick = false })

function hoverFilterEnter() {
  if (hoverFilterTimer) {
    clearTimeout(hoverFilterTimer)
    hoverFilterTimer = null
  }
}

function hoverFilterLeave() {
  // 点击打开的下拉不因 hover 离开自动关闭
  if (filterOpenedByClick) return
  hoverFilterTimer = setTimeout(() => {
    openFilter.value = null
  }, 120)
}

function toggleFilter(filter: 'status' | 'importance') {
  if (openFilter.value === filter) {
    openFilter.value = null
  } else {
    openFilter.value = filter
    filterOpenedByClick = true
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
          <span class="filter-value" :style="{ color: props.importanceFilter !== 'all' ? 'var(--text)' : 'var(--text-muted)' }"><span v-if="importanceDotClass" class="importance-dot" :class="importanceDotClass"></span>{{ importanceDisplayText }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'importance'" class="filter-dropdown" role="menu" @mouseenter="hoverFilterEnter()" @mouseleave="hoverFilterLeave()" @keydown="handleDropdownKeydown">
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === 'all'" :class="{ selected: props.importanceFilter === 'all' }" @click="emit('update:importanceFilter', 'all'); openFilter = null">{{ t('release.filter_all') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '大'" :class="{ selected: props.importanceFilter === '大' }" @click="emit('update:importanceFilter', '大'); openFilter = null"><span class="importance-dot importance-dot-high"></span>{{ t('release.importance_high') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '中'" :class="{ selected: props.importanceFilter === '中' }" @click="emit('update:importanceFilter', '中'); openFilter = null"><span class="importance-dot importance-dot-medium"></span>{{ t('release.importance_medium') }}</button>
          <button type="button" role="menuitem" :aria-selected="props.importanceFilter === '小'" :class="{ selected: props.importanceFilter === '小' }" @click="emit('update:importanceFilter', '小'); openFilter = null"><span class="importance-dot importance-dot-low"></span>{{ t('release.importance_low') }}</button>
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
  background: var(--bg-subtle);
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

.filter-value {
  display: inline-flex;
  align-items: center;
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
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-md);
  padding: 4px;
  white-space: nowrap;
}

.filter-dropdown button {
  display: flex;
  align-items: center;
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

.filter-dropdown button:hover {
  background: var(--bg-subtle);
}

.filter-dropdown button.selected {
  font-weight: 600;
  color: var(--primary);
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

/* 滚动后 sticky 元素贴顶 */
:global(.app-main.is-scrolled .log-search-row) {
  top: calc(-1 * var(--app-padding-y, 16px));
}
</style>
