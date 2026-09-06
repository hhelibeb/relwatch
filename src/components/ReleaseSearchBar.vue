<script setup lang="ts">
import { ref, computed, inject } from 'vue'
import { t } from '../i18n'
import { useDropdown } from '../composables/useDropdown'
import { track } from '../composables/useUsageTracking'
import { getSourceTypeDef, sourceTypeDefs } from '../api/source-registry'
import { flagColorByIndex, releaseFlagged } from '../utils/releaseFlag'
import { isUnreadStatus } from '../utils'
import { ShowImportanceKey } from '../injection-keys'
import type { ReleaseInfo } from '../api/releases'
import type { ReleaseFlagFilter, ReleaseImportanceFilter, ReleaseSourceFilter, ReleaseStatusFilter, ReleaseVersionFilter, ViewMode } from './releaseTypes'

const props = withDefaults(defineProps<{
  modelValue: string
  statusFilter: ReleaseStatusFilter
  importanceFilter: ReleaseImportanceFilter
  sourceFilter: ReleaseSourceFilter
  viewMode: ViewMode
  flagFilter: ReleaseFlagFilter
  versionFilter: ReleaseVersionFilter
  /** 全量 releases（未过滤）：供「未读 / 已标记」选项追加数量标注 */
  releases?: ReleaseInfo[]
  showSearch?: boolean
  count?: number
  deepSearch?: boolean
  deepSearching?: boolean
}>(), {
  releases: () => [],
  showSearch: true,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
  'update:importanceFilter': [value: ReleaseImportanceFilter]
  'update:sourceFilter': [value: ReleaseSourceFilter]
  'update:viewMode': [value: ViewMode]
  'update:flagFilter': [value: ReleaseFlagFilter]
  'update:versionFilter': [value: ReleaseVersionFilter]
  'update:deepSearch': [value: boolean]
  searchEnter: []
}>()

// ========== 筛选下拉状态 ==========
// 状态/漏斗/视图共享一个 open 状态：hover 打开互斥，点击打开的不会被 hover 移出自动关闭
// （来源筛选只在漏斗面板中：低频维度，栏上重复入口已移除，激活态由 chips + 漏斗计数展示）
const openFilter = ref<'status' | 'more' | 'view' | null>(null)
const filterDropdown = useDropdown({
  openState: openFilter,
  closedKey: null,
  hoverOpen: true,
})

// 「显示重要度」开关（App.vue provide）：关闭时漏斗面板不含重要度分组、chips 不显示重要度
const showImportance = inject(ShowImportanceKey, ref(false))

const statusDisplayText = computed(() => {
  if (props.statusFilter === 'unread') return t('release.filter_unread')
  if (props.statusFilter === 'read') return t('release.filter_read')
  return t('release.filter_all')
})

const importanceDisplayText = computed(() => {
  if (props.importanceFilter === '大') return t('release.importance_high')
  if (props.importanceFilter === '中') return t('release.importance_medium')
  if (props.importanceFilter === '小') return t('release.importance_low')
  return t('release.filter_all')
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

// ── 旗标 toggle：栏上快捷开关「仅已标记」；具体颜色与未标记在漏斗面板中选择 ──
const flagToggleActive = computed(() => props.flagFilter === 'flagged')
const flagToggleColor = computed(() => {
  if (typeof props.flagFilter === 'number') return flagColorByIndex(props.flagFilter)
  if (flagToggleActive.value) return 'var(--primary)'
  return ''
})

function toggleFlagFilter() {
  const next: ReleaseFlagFilter = flagToggleActive.value ? 'all' : 'flagged'
  emit('update:flagFilter', next)
  track('release.filter_flag')
}

const FLAG_OPTIONS: { value: number; labelKey: string }[] = [
  { value: 1, labelKey: 'release.flag_red' },
  { value: 2, labelKey: 'release.flag_orange' },
  { value: 3, labelKey: 'release.flag_yellow' },
  { value: 4, labelKey: 'release.flag_green' },
  { value: 5, labelKey: 'release.flag_blue' },
  { value: 6, labelKey: 'release.flag_purple' },
]

const versionOptions = computed(() => [
  { value: 'major' as const, label: t('release.bump_major') },
  { value: 'minor' as const, label: t('release.bump_minor') },
  { value: 'patch' as const, label: t('release.bump_patch') },
  { value: 'prerelease' as const, label: t('release.bump_prerelease') },
])

const versionDisplayText = computed(() => {
  switch (props.versionFilter) {
    case 'major': return t('release.bump_major')
    case 'minor': return t('release.bump_minor')
    case 'patch': return t('release.bump_patch')
    case 'prerelease': return t('release.bump_prerelease')
    default: return t('release.filter_all')
  }
})

// ── 选项计数标注：「未读 / 已标记」等高频选项在 label 后追加 (n)；0 条不追加 ──
const unreadCount = computed(() => props.releases.filter(r => isUnreadStatus(r.notification_status)).length)
const flaggedCount = computed(() => props.releases.filter(r => releaseFlagged(r)).length)
const withCount = (label: string, n: number) => (n > 0 ? `${label} (${n})` : label)

// ── 漏斗面板分组，单选即选即关；重要度分组受「显示重要度」开关控制。
// 栏上已有一级入口的维度（状态）排在面板末位，低频维度靠前 ──
interface MoreGroup {
  key: string
  title: string
  current: unknown
  options: { value: unknown; label: string; dotClass?: string; flagColor?: string; iconHref?: string; emphasis?: boolean }[]
  select: (value: unknown) => void
}

const moreGroups = computed<MoreGroup[]>(() => [
  {
    key: 'source',
    title: t('tab.source'),
    current: props.sourceFilter,
    options: [
      { value: 'all', label: t('release.filter_all') },
      ...sourceTypeDefs.map(def => ({ value: def.type, label: t(def.titleKey), iconHref: def.icon })),
    ],
    select: (v) => selectSourceFilter(v as ReleaseSourceFilter),
  },
  {
    key: 'version',
    title: t('release.bump'),
    current: props.versionFilter,
    options: [{ value: 'all', label: t('release.filter_all') }, ...versionOptions.value],
    select: (v) => selectVersionFilter(v as ReleaseVersionFilter),
  },
  {
    key: 'importance',
    title: t('tab.importance'),
    current: props.importanceFilter,
    options: [
      { value: 'all', label: t('release.filter_all') },
      { value: '大', label: t('release.importance_high'), dotClass: 'importance-dot-high' },
      { value: '中', label: t('release.importance_medium'), dotClass: 'importance-dot-medium' },
      { value: '小', label: t('release.importance_low'), dotClass: 'importance-dot-low' },
    ],
    select: (v) => selectImportanceFilter(v as ReleaseImportanceFilter),
  },
  {
    key: 'flag',
    title: t('release.flag'),
    current: props.flagFilter,
    options: [
      { value: 'all', label: t('release.filter_all') },
      { value: 'flagged', label: withCount(t('release.flag_flagged'), flaggedCount.value) },
      { value: 'unflagged', label: t('release.flag_unflagged') },
      ...FLAG_OPTIONS.map(o => ({ value: o.value, label: t(o.labelKey), flagColor: flagColorByIndex(o.value) ?? undefined })),
    ],
    select: (v) => selectFlagFilter(v as ReleaseFlagFilter),
  },
  {
    key: 'status',
    title: t('tab.status'),
    current: props.statusFilter,
    options: [
      { value: 'all', label: t('release.filter_all') },
      { value: 'unread', label: withCount(t('release.filter_unread'), unreadCount.value), emphasis: unreadCount.value > 0 },
      { value: 'read', label: t('release.filter_read') },
    ],
    select: (v) => selectStatusFilter(v as ReleaseStatusFilter),
  },
])

// 「显示重要度」关闭时漏斗面板移除重要度分组
const visibleGroups = computed<MoreGroup[]>(() =>
  moreGroups.value.filter(group => showImportance.value || group.key !== 'importance'))

// ── 漏斗面板双列：保持语义顺序，按 ceil(n/2) 动态分列（5 组=3+2，关闭重要度后 4 组=2+2），
// 两列各自独立堆叠，避免单列过长必须滚动；
// 列容器 role=presentation 对读屏穿透，menu 的可访问子元素仍为 group ──
const moreGroupColumns = computed<MoreGroup[][]>(() => {
  const groups = visibleGroups.value
  const half = Math.ceil(groups.length / 2)
  return [groups.slice(0, half), groups.slice(half)]
})

// ── 激活筛选 chips：与漏斗计数共用，点击 chip 移除对应维度 ──
interface ActiveChip {
  key: string
  label: string
  color?: string
  iconHref?: string
  dot?: string
  clear: () => void
}

const activeChips = computed<ActiveChip[]>(() => {
  const chips: ActiveChip[] = []
  if (props.statusFilter !== 'all') {
    chips.push({ key: 'status', label: statusDisplayText.value, clear: () => emit('update:statusFilter', 'all') })
  }
  if (props.versionFilter !== 'all') {
    chips.push({ key: 'version', label: versionDisplayText.value, clear: () => emit('update:versionFilter', 'all') })
  }
  if (props.flagFilter !== 'all') {
    const label = props.flagFilter === 'flagged'
      ? t('release.flag_flagged')
      : props.flagFilter === 'unflagged'
        ? t('release.flag_unflagged')
        : t(FLAG_OPTIONS[(props.flagFilter as number) - 1].labelKey)
    chips.push({
      key: 'flag',
      label,
      color: typeof props.flagFilter === 'number' ? flagColorByIndex(props.flagFilter) ?? 'var(--primary)' : 'var(--primary)',
      clear: () => emit('update:flagFilter', 'all'),
    })
  }
  if (props.sourceFilter !== 'all') {
    chips.push({ key: 'source', label: sourceDisplayText.value, iconHref: sourceDef.value?.icon, clear: () => emit('update:sourceFilter', 'all') })
  }
  if (showImportance.value && props.importanceFilter !== 'all') {
    const dotMap: Record<string, string> = { 大: 'var(--danger)', 中: 'var(--warning)', 小: 'var(--success)' }
    chips.push({ key: 'importance', label: importanceDisplayText.value, dot: dotMap[props.importanceFilter], clear: () => emit('update:importanceFilter', 'all') })
  }
  return chips
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

function selectFlagFilter(value: ReleaseFlagFilter) {
  emit('update:flagFilter', value)
  filterDropdown.close()
  track('release.filter_flag')
}

function selectVersionFilter(value: ReleaseVersionFilter) {
  emit('update:versionFilter', value)
  filterDropdown.close()
  track('release.filter_bump')
}

function selectViewMode(value: ViewMode) {
  emit('update:viewMode', value)
  filterDropdown.close()
  track('release.view_' + value)
}

// 一键清空漏斗面板管理的全部筛选维度（不动搜索词：输入框自带清空按钮）
function clearAllFilters() {
  emit('update:statusFilter', 'all')
  emit('update:versionFilter', 'all')
  emit('update:flagFilter', 'all')
  emit('update:sourceFilter', 'all')
  emit('update:importanceFilter', 'all')
  filterDropdown.close()
  track('release.filter_clear_all')
}
</script>

<template>
  <div class="log-search-sticky">
    <div class="log-search-row">
      <div v-if="props.showSearch" class="input-clear-wrap">
        <input
          :value="modelValue"
          :placeholder="t('release.search')"
          class="search-input"
          @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
          @keydown.enter.prevent="onSearchEnter"
        />
        <span v-if="count !== undefined" class="release-count" :class="{ 'has-clear': modelValue !== '' }" :title="t('release.versions', String(count))">({{ count }})</span>
        <button v-if="modelValue" type="button" class="deep-search-btn" :class="{ active: deepSearch }" :disabled="deepSearching" :title="t('release.deep_search_hint')" @click="emit('update:deepSearch', !deepSearch)">{{ t('release.deep_search') }}</button>
        <button v-if="modelValue" type="button" class="input-clear-btn" :title="t('input.clear')" @click="emit('update:modelValue', '')">✕</button>
      </div>
      <div class="filter-group" @mouseleave="filterDropdown.hoverLeave()">
        <!-- 漏斗居首：全量筛选入口；旗标/状态为单一维度快捷筛选，排其后 -->
        <div class="filter-field" @mouseenter="filterDropdown.hoverEnter('more')">
          <button type="button" class="filter-trigger filter-more-trigger" :aria-expanded="openFilter === 'more'" aria-haspopup="menu" :title="t('release.filter_more')" @click="filterDropdown.toggle($event, 'more')" @keydown="filterDropdown.handleTriggerKeydown($event, 'more')">
            <svg class="filter-more-icon"><use href="/icons.svg#filter-icon"/></svg>
            <span v-if="activeChips.length" class="filter-more-count">{{ activeChips.length }}</span>
          </button>
          <div v-if="openFilter === 'more'" class="dropdown-panel filter-dropdown filter-more-panel" role="menu" @mouseenter="filterDropdown.hoverEnter('more')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
            <div class="filter-more-columns">
              <div v-for="(col, ci) in moreGroupColumns" :key="ci" class="filter-more-col" role="presentation">
                <template v-for="group in col" :key="group.key">
                  <!-- menu 的合法子元素只有 menuitem/group/separator：分组标题经 role=group + aria-label 供读屏识别，视觉标题 aria-hidden 防重复朗读 -->
                  <div class="filter-group-block" role="group" :aria-label="group.title">
                    <div class="filter-group-title" aria-hidden="true">{{ group.title }}</div>
                    <button
                      v-for="opt in group.options"
                      :key="String(opt.value)"
                      type="button"
                      role="menuitem"
                      :aria-selected="group.current === opt.value"
                      :class="{ selected: group.current === opt.value, 'filter-option-emphasis': opt.emphasis }"
                      @click="group.select(opt.value)"
                    >
                      <span v-if="opt.flagColor" class="filter-type-icon" :style="{ color: opt.flagColor }"><svg><use href="/icons.svg#flag-tag-icon"/></svg></span>
                      <span v-else-if="opt.iconHref" class="filter-type-icon"><svg><use :href="opt.iconHref"/></svg></span>
                      <span v-else-if="opt.dotClass" class="importance-dot" :class="opt.dotClass"></span>
                      {{ opt.label }}
                    </button>
                  </div>
                </template>
              </div>
            </div>
            <!-- 仅在有激活筛选时出现：一键重置五个维度（不动搜索词） -->
            <button v-if="activeChips.length" type="button" role="menuitem" class="filter-clear-all" @click="clearAllFilters">{{ t('release.filter_clear_all') }}</button>
          </div>
        </div>
        <div class="filter-divider"></div>
        <div class="filter-field">
          <button type="button" class="filter-trigger filter-flag-toggle" :class="{ active: flagToggleActive || typeof props.flagFilter === 'number' }" :aria-pressed="flagToggleActive" :title="t('release.flag')" @click="toggleFlagFilter">
            <svg class="filter-flag-icon" :style="flagToggleColor ? { color: flagToggleColor } : undefined"><use href="/icons.svg#flag-tag-icon"/></svg>
          </button>
        </div>
        <div class="filter-divider"></div>
        <div class="filter-field" @mouseenter="filterDropdown.hoverEnter('status')">
          <button type="button" class="filter-trigger" :aria-expanded="openFilter === 'status'" aria-haspopup="menu" @click="filterDropdown.toggle($event, 'status')" @keydown="filterDropdown.handleTriggerKeydown($event, 'status')">
            <span class="filter-label">{{ t('tab.status') }}</span>
            <span class="filter-value" :style="{ color: props.statusFilter === 'unread' ? 'var(--primary)' : props.statusFilter === 'read' ? 'var(--success)' : 'var(--text-muted)' }">{{ statusDisplayText }}</span>
            <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
          </button>
          <div v-if="openFilter === 'status'" class="dropdown-panel filter-dropdown" role="menu" @mouseenter="filterDropdown.hoverEnter('status')" @mouseleave="filterDropdown.hoverLeave()" @keydown="filterDropdown.handleDropdownKeydown">
            <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'all'" :class="{ selected: props.statusFilter === 'all' }" @click="selectStatusFilter('all')">{{ t('release.filter_all') }}</button>
            <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'unread'" :class="{ selected: props.statusFilter === 'unread', 'filter-option-emphasis': unreadCount > 0 }" @click="selectStatusFilter('unread')">{{ withCount(t('release.filter_unread'), unreadCount) }}</button>
            <button type="button" role="menuitem" :aria-selected="props.statusFilter === 'read'" :class="{ selected: props.statusFilter === 'read' }" @click="selectStatusFilter('read')">{{ t('release.filter_read') }}</button>
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
      <slot />
    </div>
    <div v-if="activeChips.length" class="filter-chips-row">
      <button v-for="chip in activeChips" :key="chip.key" type="button" class="filter-chip" :title="t('release.filter_reset')" @click="chip.clear()">
        <span v-if="chip.color" class="filter-chip-flag" :style="{ color: chip.color }"><svg><use href="/icons.svg#flag-tag-icon"/></svg></span>
        <span v-else-if="chip.iconHref" class="filter-type-icon"><svg><use :href="chip.iconHref"/></svg></span>
        <span v-else-if="chip.dot" class="filter-chip-dot" :style="{ background: chip.dot }"></span>
        <span>{{ chip.label }}</span>
        <span class="filter-chip-x">✕</span>
      </button>
    </div>
  </div>
</template>
<style scoped>
/* 版本计数徽标：绝对定位在搜索框右内侧，不占用 flex 布局宽度。
   默认贴右缘；有输入（显示清空按钮）时让位到清空按钮左侧，避免重叠。 */
.release-count {
  position: absolute;
  right: 10px;
  top: 50%;
  transform: translateY(-50%);
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
  pointer-events: none;
  transition: right 0.15s ease;
}

.release-count.has-clear {
  right: 84px;
}

/* 深度搜索切换按钮：位于清空按钮左侧，仅在有搜索词时显示。
   开启态高亮为 primary 色；构建期间禁用防止重复触发。 */
.deep-search-btn {
  position: absolute;
  right: 32px;
  top: 50%;
  height: 22px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
  white-space: nowrap;
  transform: translateY(-50%);
}

.deep-search-btn:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.deep-search-btn.active {
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
  color: var(--primary-soft-text);
}

.deep-search-btn:disabled {
  opacity: 0.5;
  cursor: default;
}

/* 为计数徽标 + 清空按钮 + 深度按钮预留右侧空间，避免输入长文本时重叠 */
.input-clear-wrap .search-input {
  padding-right: 130px;
}

/* 旗标 toggle：icon-only 按钮；激活/选中具体颜色时着色 */
.filter-flag-toggle {
  padding: 6px 9px;
}

.filter-flag-icon {
  width: 14px;
  height: 14px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.filter-flag-toggle.active .filter-flag-icon {
  color: var(--primary);
}

/* 漏斗触发按钮：icon-only + 激活筛选计数徽标 */
.filter-more-trigger {
  gap: 4px;
}

.filter-more-icon {
  width: 14px;
  height: 14px;
  color: var(--text);
  flex-shrink: 0;
}

.filter-more-count {
  min-width: 15px;
  height: 15px;
  padding: 0 4px;
  border-radius: 999px;
  background: var(--primary);
  color: #fff;
  font-size: 10px;
  line-height: 15px;
  text-align: center;
  font-weight: 600;
}

/* 漏斗面板：双列分组，比常规筛选下拉更宽 */
.filter-more-panel {
  left: auto;
  right: 0;
  transform: none;
  min-width: 300px;
  max-height: min(600px, calc(100vh - 120px));
  overflow-y: auto;
}

/* 未读有存量时选项加粗提示（仅未读；分组不加粗） */
.filter-option-emphasis {
  font-weight: 600;
}

/* 面板底部「清空全部筛选」：通栏 + 顶部分隔线，弱化常态、hover 提示危险 */
.filter-clear-all {
  display: block;
  width: calc(100% - 20px);
  margin: 6px 10px 2px;
  padding: 6px 10px;
  border-top: 1px solid var(--border);
  border-radius: 0;
  font-size: 12px;
  color: var(--text-muted);
  text-align: center;
}

.filter-clear-all:hover {
  color: var(--danger);
}

.filter-more-columns {
  display: flex;
  align-items: flex-start;
}

.filter-more-col {
  flex: 1 1 0;
  min-width: 0;
}

/* 列间细分隔线，右列左移出留白 */
.filter-more-col + .filter-more-col {
  margin-left: 8px;
  padding-left: 10px;
  border-left: 1px solid var(--border);
}

.filter-group-block + .filter-group-block {
  margin-top: 4px;
}

.filter-group-title {
  padding: 3px 14px 2px;
  font-size: 10px;
  color: var(--text-muted);
  user-select: none;
}

/* 激活筛选 chips 行 */
.filter-chips-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

.filter-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  font-size: 12px;
  line-height: 1.5;
  cursor: pointer;
  transition: background 0.1s, border-color 0.1s;
}

.filter-chip:hover {
  background: var(--bg-subtle);
  border-color: var(--border-strong);
}

.filter-chip-flag {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.filter-chip-flag svg {
  width: 12px;
  height: 12px;
}

.filter-chip-dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  flex-shrink: 0;
}

.filter-chip-x {
  color: var(--text-muted);
  font-size: 10px;
  line-height: 1;
}

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
