<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import type { ReleaseInfo } from '../api/releases'
import {
  batchDeleteReleases,
  batchSetReleaseFlag,
  batchUpdateReleaseStatus,
} from '../api/releases'
import { isReadStatus, isUnreadStatus, filterReleaseIndices } from '../utils'
import ReleaseAggregatedList from './ReleaseAggregatedList.vue'
import ReleaseCalendar from './ReleaseCalendar.vue'
import ReleaseDateDetail from './ReleaseDateDetail.vue'
import ReleaseDetailModal from './ReleaseDetailModal.vue'
import ReleaseSearchBar from './ReleaseSearchBar.vue'
import ReleaseSimpleList from './ReleaseSimpleList.vue'
import type { ReleaseStatusFilter } from './releaseTypes'
import { FLAG_ITEMS, flagColorByIndex, releaseFlagged } from '../utils/releaseFlag'
import { clampCalendarMonth, toDateKey } from '../utils/dateKey'
import { useBulkSelection } from '../composables/useBulkSelection'
import { useReleaseDeepSearch } from '../composables/useReleaseDeepSearch'
import { useReleaseDetailNav } from '../composables/useReleaseDetailNav'
import { useListKeyboardNav } from '../composables/useListKeyboardNav'
import { track } from '../composables/useUsageTracking'
import { confirmAction, extractErrorMessage } from '../services/feedback'
import { useUiStore } from '../stores/ui'
import { useSettingsStore } from '../stores/settings'
import { useReleaseFilterStore } from '../stores/releaseFilter'
import { t } from '../i18n'
import ErrorBlock from './common/ErrorBlock.vue'

type ReleaseSimpleListHandle = InstanceType<typeof ReleaseSimpleList> & {
  focusReleaseId: (id: number) => boolean
  selectFirst: () => boolean
  moveSelection: (delta: number) => void
  openSelected: () => void
}
type AggregatedListInstance = InstanceType<typeof ReleaseAggregatedList> & {
  expandAll: () => void
}

const props = withDefaults(defineProps<{
  releases: ReleaseInfo[]
  search?: string
  statusFilter?: ReleaseStatusFilter
  focusTarget?: number | null
  focusToken?: number
  loading?: boolean
  loadError?: string
  active?: boolean
}>(), {
  search: '',
  statusFilter: 'all',
  focusTarget: null,
  focusToken: 0,
  loading: false,
  loadError: '',
  active: false,
})

const emit = defineEmits<{
  update: []
  'update:search': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
  'focus-consumed': []
  'focus-not-found': []
  reload: []
}>()

const ui = useUiStore()
const settingsStore = useSettingsStore()
const filterStore = useReleaseFilterStore()
const { showImportance, autoReadDetail } = storeToRefs(settingsStore)
const {
  viewMode,
  importanceFilter,
  sourceFilter,
  flagFilter,
  versionFilter,
} = storeToRefs(filterStore)

const selectedDate = ref<string | null>(null)
const calendarYear = ref(new Date().getFullYear())
const calendarMonth = ref(new Date().getMonth() + 1)
const simpleList = ref<ReleaseSimpleListHandle | null>(null)
const aggregatedList = ref<AggregatedListInstance | null>(null)
const searchBar = ref<InstanceType<typeof ReleaseSearchBar> | null>(null)

const releaseSearch = computed({
  get: () => props.search,
  set: (value: string) => {
    filterStore.search = value
    emit('update:search', value)
  },
})

const statusFilter = computed({
  get: () => props.statusFilter,
  set: (value: ReleaseStatusFilter) => {
    filterStore.statusFilter = value
    emit('update:statusFilter', value)
  },
})

watch(
  () => props.statusFilter,
  (val, oldVal) => {
    if (filterStore.statusFilter === val) return
    if (oldVal === undefined) {
      filterStore.runWithoutPersist(() => {
        filterStore.statusFilter = val
      })
    } else {
      filterStore.statusFilter = val
    }
  },
  { immediate: true },
)

// ── 通知下钻定位消费 ──
let lastConsumedToken = -1

function waitForListReady(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

async function doFocusTarget(targetId: number) {
  const token = props.focusToken
  const release = props.releases.find((r) => r.id === targetId)
  if (!release) {
    if (props.releases.length > 0 && token > lastConsumedToken) {
      lastConsumedToken = token
      emit('focus-not-found')
    }
    return
  }

  filterStore.runWithoutPersist(() => {
    releaseSearch.value = ''
    statusFilter.value = 'all'
    importanceFilter.value = 'all'
    sourceFilter.value = 'all'
    flagFilter.value = 'all'
    versionFilter.value = 'all'
    viewMode.value = 'simple'
    selectedDate.value = null
  })
  await waitForListReady()

  if (props.focusTarget !== targetId || props.focusToken !== token) return

  let handled = simpleList.value?.focusReleaseId?.(targetId) ?? false
  if (!handled) {
    await nextTick()
    if (props.focusTarget !== targetId || props.focusToken !== token) return
    handled = simpleList.value?.focusReleaseId?.(targetId) ?? false
  }
  lastConsumedToken = token
  if (handled) emit('focus-consumed')
  else emit('focus-not-found')
}

function consumeFocusIfPending() {
  const target = props.focusTarget
  if (target === null || props.focusToken <= lastConsumedToken) return
  void doFocusTarget(target)
}

watch(() => props.focusToken, consumeFocusIfPending, { flush: 'post' })
watch(() => props.releases, consumeFocusIfPending, { flush: 'post' })

const hasActiveFilter = computed(() =>
  releaseSearch.value.trim() !== ''
  || statusFilter.value !== 'all'
  || (showImportance.value && importanceFilter.value !== 'all')
  || sourceFilter.value !== 'all'
  || flagFilter.value !== 'all'
  || versionFilter.value !== 'all',
)

const showLoadError = computed(() => props.loadError !== '' && props.releases.length === 0)

function clearSearch() {
  releaseSearch.value = ''
}

function clearFilters() {
  statusFilter.value = 'all'
  importanceFilter.value = 'all'
  sourceFilter.value = 'all'
  flagFilter.value = 'all'
  versionFilter.value = 'all'
}

const {
  deepSearch,
  bodyIndex,
  deepSearching,
  bodyTruncated,
  onDeepSearchToggle,
  enableDeepSearch,
} = useReleaseDeepSearch({
  releases: () => props.releases,
  searchQuery: releaseSearch,
})

const filteredReleases = computed(() => {
  let list = props.releases
  const q = releaseSearch.value.trim()
  if (q) {
    const picked = filterReleaseIndices(list, q, deepSearch.value ? bodyIndex.value : null)
    list = picked.map((i) => list[i])
  }
  if (statusFilter.value === 'unread') {
    list = list.filter((release) => isUnreadStatus(release.notification_status, release.snooze_until))
  } else if (statusFilter.value === 'read') {
    list = list.filter((release) => isReadStatus(release.notification_status))
  }
  if (showImportance.value && importanceFilter.value !== 'all') {
    list = list.filter((release) => release.ai_importance === importanceFilter.value)
  }
  if (sourceFilter.value !== 'all') {
    list = list.filter((release) => release.source_type === sourceFilter.value)
  }
  if (flagFilter.value === 'flagged') {
    list = list.filter(releaseFlagged)
  } else if (flagFilter.value === 'unflagged') {
    list = list.filter((release) => !releaseFlagged(release))
  } else if (flagFilter.value !== 'all') {
    list = list.filter((release) => release.flag === flagFilter.value)
  }
  if (versionFilter.value !== 'all') {
    if (versionFilter.value === 'prerelease') {
      list = list.filter((release) => release.prerelease)
    } else {
      list = list.filter((release) => release.version_bump === versionFilter.value)
    }
  }
  return list
})

async function handleSearchEnter() {
  if (viewMode.value === 'aggregated') {
    await nextTick()
    aggregatedList.value?.expandAll()
    return
  }
  if (viewMode.value === 'simple') {
    await nextTick()
    if (simpleList.value?.selectFirst()) {
      const active = document.activeElement
      if (active instanceof HTMLInputElement) active.blur()
    }
  }
}

function backToCalendar() { selectedDate.value = null }
function shiftMonth(delta: number) {
  const next = clampCalendarMonth(calendarYear.value, calendarMonth.value + delta)
  calendarYear.value = next.year
  calendarMonth.value = next.month
}
function prevMonth() { shiftMonth(-1) }
function nextMonth() { shiftMonth(1) }
function jumpCalendar(year: number, month: number) {
  const next = clampCalendarMonth(year, month)
  calendarYear.value = next.year
  calendarMonth.value = next.month
  selectedDate.value = null
}

watch(viewMode, () => { selectedDate.value = null })

// ── 详情弹窗序列导航（抽离到 useReleaseDetailNav）──
const {
  detailReleaseId,
  detailSequenceIds,
  detailIndex,
  detailRelease,
  openReleaseDetail,
  closeReleaseDetail,
  navigateReleaseDetail,
} = useReleaseDetailNav({
  releases: () => props.releases,
  autoReadDetail,
  onUpdated: () => emit('update'),
})

function focusSearch() {
  closeReleaseDetail()
  void nextTick(() => searchBar.value?.focusInput())
}
defineExpose({ focusSearch })

// ── 简单视图键盘导航（抽离到 useListKeyboardNav）──
useListKeyboardNav({
  isActive: () => props.active && detailReleaseId.value === null && viewMode.value === 'simple',
  onMove: (delta) => simpleList.value?.moveSelection(delta),
  onOpenSelected: () => simpleList.value?.openSelected(),
})

// ── 批量操作 ──
const BULK_FLAG_SWATCHES = FLAG_ITEMS
const bulkBusy = ref(false)
const flagPopoverOpen = ref(false)
const canSelect = computed(() => viewMode.value !== 'calendar' || selectedDate.value !== null)

const visibleSelectionReleases = computed<ReleaseInfo[]>(() => {
  if (viewMode.value === 'calendar') {
    if (selectedDate.value === null) return []
    return filteredReleases.value
      .filter((r) => toDateKey(new Date(r.published_at)) === selectedDate.value)
      .sort((a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime())
  }
  if (viewMode.value === 'simple') {
    return [...filteredReleases.value].sort(
      (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime(),
    )
  }
  return filteredReleases.value
})

const unreadInViewIds = computed(() =>
  filteredReleases.value
    .filter((r) => isUnreadStatus(r.notification_status, r.snooze_until))
    .map((r) => r.id),
)

const visibleSelectionIds = computed(() => visibleSelectionReleases.value.map((r) => r.id))

const bulkSelection = useBulkSelection<number>({
  visibleIds: visibleSelectionIds,
  canSelect,
  active: computed(() => props.active),
  canHandleEscape: () => detailReleaseId.value === null,
  onEscapeBeforeExit: () => {
    if (!flagPopoverOpen.value) return false
    flagPopoverOpen.value = false
    return true
  },
  onExit: () => { flagPopoverOpen.value = false },
})

const {
  selectionMode,
  selectedIds,
  selectedCount,
  allVisibleSelected,
  exitSelectionMode,
  clearSelected,
} = bulkSelection

function toggleSelectionMode() {
  if (!bulkSelection.toggleSelectionMode()) return
  track('release.select')
}

function toggleSelectRelease(id: number, shiftKey = false) {
  bulkSelection.toggle(id, shiftKey)
}

function selectAllVisible() {
  bulkSelection.selectAllVisible()
  track('release.bulk_select_all')
}

async function runBulkAction(
  action: () => Promise<number>,
  toastKey: (count: number) => string,
  errPrefixKey: string,
  onSuccess?: () => void,
) {
  bulkBusy.value = true
  try {
    const affected = await action()
    onSuccess?.()
    if (affected > 0) ui.showToast(toastKey(affected))
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t(errPrefixKey) + extractErrorMessage(e))
  } finally {
    bulkBusy.value = false
  }
}

async function confirmAndMarkRead(ids: number[], confirmMsg: string, titleKey: string, trackEvent: string) {
  if (ids.length === 0 || bulkBusy.value) return
  const confirmed = await confirmAction(confirmMsg, { title: t(titleKey), kind: 'info' })
  if (!confirmed) return
  track(trackEvent)
  await runBulkAction(
    () => batchUpdateReleaseStatus(ids, 'clicked'),
    (n) => t('release.bulk_marked_read', String(n)),
    'release.status_failed',
  )
}

async function handleMarkAllRead() {
  const ids = unreadInViewIds.value
  const msg = hasActiveFilter.value
    ? t('release.mark_all_read_filtered_confirm', String(ids.length))
    : t('release.mark_all_read_confirm', String(ids.length))
  await confirmAndMarkRead(ids, msg, 'release.mark_all_read', 'release.mark_all_read')
}

async function handleMarkGroupRead(groupName: string, unreadIds: number[]) {
  await confirmAndMarkRead(
    unreadIds,
    t('release.mark_source_read_confirm', groupName, String(unreadIds.length)),
    'release.mark_source_read',
    'release.mark_source_read',
  )
}

async function handleBulkStatus(status: 'clicked' | 'ignored') {
  const ids = [...selectedIds.value]
  if (ids.length === 0 || bulkBusy.value) return
  flagPopoverOpen.value = false
  track(status === 'clicked' ? 'release.bulk_read' : 'release.bulk_ignore')
  await runBulkAction(
    () => batchUpdateReleaseStatus(ids, status),
    (n) => t(status === 'clicked' ? 'release.bulk_marked_read' : 'release.bulk_ignored', String(n)),
    'release.status_failed',
  )
}

async function handleBulkFlag(flag: number) {
  const ids = [...selectedIds.value]
  flagPopoverOpen.value = false
  if (ids.length === 0 || bulkBusy.value) return
  track('release.bulk_flag')
  await runBulkAction(
    () => batchSetReleaseFlag(ids, flag),
    (n) => t(flag === 0 ? 'release.bulk_flag_cleared' : 'release.bulk_flagged', String(n)),
    'release.flag_failed',
  )
}

async function handleBulkDelete() {
  const ids = [...selectedIds.value]
  if (ids.length === 0 || bulkBusy.value) return
  flagPopoverOpen.value = false
  const confirmed = await confirmAction(t('release.bulk_delete_confirm', String(ids.length)), {
    title: t('release.bulk_delete'),
    kind: 'warning',
  })
  if (!confirmed) return
  track('release.bulk_delete')
  await runBulkAction(
    () => batchDeleteReleases(ids),
    (n) => t('release.bulk_deleted', String(n)),
    'release.delete_failed',
    clearSelected,
  )
}

function closeFlagPopover() { flagPopoverOpen.value = false }
onMounted(() => document.addEventListener('click', closeFlagPopover))
onUnmounted(() => document.removeEventListener('click', closeFlagPopover))
</script>

<template>
  <section class="tab-content">
    <ReleaseSearchBar
      ref="searchBar"
      v-model="releaseSearch"
      v-model:status-filter="statusFilter"
      v-model:importance-filter="importanceFilter"
      v-model:source-filter="sourceFilter"
      v-model:view-mode="viewMode"
      v-model:flag-filter="flagFilter"
      v-model:version-filter="versionFilter"
      :releases="releases"
      :count="filteredReleases.length"
      :deep-search="deepSearch"
      :deep-searching="deepSearching"
      :body-truncated="bodyTruncated"
      :unread-in-view="unreadInViewIds.length"
      :selection-mode="selectionMode"
      :can-select="canSelect"
      :bulk-busy="bulkBusy"
      @update:deep-search="onDeepSearchToggle"
      @search-enter="handleSearchEnter"
      @mark-all-read="handleMarkAllRead"
      @toggle-select="toggleSelectionMode"
    >
      <template #below>
        <Transition name="bulk-bar">
          <div v-if="selectionMode" class="release-bulk-bar">
            <span class="bulk-count" :class="{ active: selectedCount > 0 }">
              {{ t('release.bulk_count', String(selectedCount)) }}
            </span>
            <button
              type="button"
              class="bulk-link-btn"
              :disabled="visibleSelectionReleases.length === 0 || allVisibleSelected"
              @click="selectAllVisible"
            >{{ t('release.bulk_select_all') }}</button>
            <button
              v-if="selectedCount > 0"
              type="button"
              class="bulk-link-btn"
              @click="clearSelected"
            >{{ t('release.bulk_clear') }}</button>
            <span class="bulk-sep" aria-hidden="true"></span>
            <button
              type="button"
              class="bulk-action-btn"
              :disabled="selectedCount === 0 || bulkBusy"
              @click="handleBulkStatus('clicked')"
            >
              <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#check-icon" /></svg>
              <span>{{ t('release.bulk_read') }}</span>
            </button>
            <button
              type="button"
              class="bulk-action-btn"
              :disabled="selectedCount === 0 || bulkBusy"
              @click="handleBulkStatus('ignored')"
            >
              <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#ignore-icon" /></svg>
              <span>{{ t('release.bulk_ignore') }}</span>
            </button>
            <div class="bulk-flag-wrap" @click.stop>
              <button
                type="button"
                class="bulk-action-btn"
                :class="{ active: flagPopoverOpen }"
                :disabled="selectedCount === 0 || bulkBusy"
                :aria-expanded="flagPopoverOpen"
                aria-haspopup="menu"
                @click="flagPopoverOpen = !flagPopoverOpen"
              >
                <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#flag-tag-icon" /></svg>
                <span>{{ t('release.bulk_flag') }}</span>
                <svg class="bulk-flag-caret" width="10" height="10" aria-hidden="true"><use href="/icons.svg#chevron-down-icon" /></svg>
              </button>
              <div v-if="flagPopoverOpen" class="dropdown-panel bulk-flag-popover" role="menu">
                <div class="bulk-flag-swatches">
                  <button
                    v-for="swatch in BULK_FLAG_SWATCHES"
                    :key="swatch.value"
                    type="button"
                    role="menuitem"
                    class="bulk-flag-swatch"
                    :style="{ color: flagColorByIndex(swatch.value) ?? undefined }"
                    :title="t(swatch.labelKey)"
                    :aria-label="t(swatch.labelKey)"
                    @click="handleBulkFlag(swatch.value)"
                  >
                    <svg width="15" height="15" aria-hidden="true"><use href="/icons.svg#flag-tag-icon" /></svg>
                  </button>
                </div>
                <button
                  type="button"
                  role="menuitem"
                  class="bulk-flag-clear"
                  @click="handleBulkFlag(0)"
                >{{ t('release.flag_clear') }}</button>
              </div>
            </div>
            <div class="bulk-right">
              <button
                type="button"
                class="bulk-action-btn bulk-action-danger"
                :disabled="selectedCount === 0 || bulkBusy"
                @click="handleBulkDelete"
              >
                <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#trash-icon" /></svg>
                <span>{{ t('release.bulk_delete') }}</span>
              </button>
              <button
                type="button"
                class="bulk-close-btn"
                :title="t('release.select_cancel')"
                :aria-label="t('release.select_cancel')"
                @click="exitSelectionMode"
              >✕</button>
            </div>
          </div>
        </Transition>
      </template>
    </ReleaseSearchBar>

    <ErrorBlock
      v-if="showLoadError"
      :message="props.loadError"
      :retry-label="t('common.retry')"
      @retry="emit('reload')"
    />

    <ReleaseSimpleList
      v-else-if="viewMode === 'simple'"
      ref="simpleList"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      :has-search-query="releaseSearch.trim() !== ''"
      :deep-search="deepSearch"
      :loading="props.loading"
      :search-query="releaseSearch"
      :body-index="deepSearch ? bodyIndex : null"
      :selectable="selectionMode"
      :selected-ids="selectedIds"
      @enable-deep="enableDeepSearch"
      @clear-search="clearSearch"
      @clear-filters="clearFilters"
      @update="emit('update')"
      @open-detail="openReleaseDetail"
      @toggle-select="toggleSelectRelease"
    />

    <ReleaseAggregatedList
      v-else-if="viewMode === 'aggregated'"
      ref="aggregatedList"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      :has-search-query="releaseSearch.trim() !== ''"
      :loading="props.loading"
      :search-query="releaseSearch"
      :body-index="deepSearch ? bodyIndex : null"
      :selectable="selectionMode"
      :selected-ids="selectedIds"
      @clear-search="clearSearch"
      @clear-filters="clearFilters"
      @update="emit('update')"
      @open-detail="openReleaseDetail"
      @toggle-select="toggleSelectRelease"
      @mark-group-read="handleMarkGroupRead"
    />

    <template v-else-if="viewMode === 'calendar'">
      <ReleaseDateDetail
        v-if="selectedDate !== null"
        :selected-date="selectedDate"
        :releases="filteredReleases"
        :search-query="releaseSearch"
        :body-index="deepSearch ? bodyIndex : null"
        :selectable="selectionMode"
        :selected-ids="selectedIds"
        @back="backToCalendar"
        @update="emit('update')"
        @open-detail="openReleaseDetail"
        @toggle-select="toggleSelectRelease"
      />
      <ReleaseCalendar
        v-else
        :releases="filteredReleases"
        :year="calendarYear"
        :month="calendarMonth"
        @prev-month="prevMonth"
        @next-month="nextMonth"
        @jump-to="jumpCalendar"
        @select-date="selectedDate = $event"
      />
    </template>

    <ReleaseDetailModal
      v-if="detailRelease"
      :release="detailRelease"
      :position="detailIndex + 1"
      :total="detailSequenceIds.length"
      :has-prev="detailIndex > 0"
      :has-next="detailIndex >= 0 && detailIndex < detailSequenceIds.length - 1"
      :search-query="releaseSearch"
      @close="closeReleaseDetail"
      @navigate="navigateReleaseDetail"
      @update="emit('update')"
    />
  </section>
</template>

<style scoped>
.release-bulk-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 8px;
  padding: 5px 8px 5px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  font-size: var(--fs-meta);
}

.bulk-count {
  color: var(--text-muted);
  font-weight: 500;
  white-space: nowrap;
  margin-right: 4px;
}

.bulk-count.active {
  color: var(--text);
  font-weight: 600;
}

.bulk-link-btn {
  padding: 3px 6px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--primary);
  font-size: var(--fs-meta);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--dur-hover) var(--ease-out), opacity var(--dur-hover) var(--ease-out);
}

.bulk-link-btn:hover:not(:disabled) {
  background: var(--primary-soft-bg);
}

.bulk-link-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.bulk-sep {
  width: 1px;
  height: 14px;
  margin: 0 4px;
  background: var(--border);
  flex-shrink: 0;
}

.bulk-action-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 8px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: var(--fs-meta);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out), opacity var(--dur-hover) var(--ease-out);
}

.bulk-action-btn:hover:not(:disabled),
.bulk-action-btn.active {
  background: var(--hover-surface);
}

.bulk-action-btn:disabled {
  opacity: 0.38;
  cursor: not-allowed;
}

.bulk-action-icon {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  color: var(--text-muted);
}

.bulk-action-btn:hover:not(:disabled) .bulk-action-icon {
  color: var(--text);
}

.bulk-action-danger {
  color: var(--danger);
}

.bulk-action-danger .bulk-action-icon {
  color: var(--danger);
}

.bulk-action-danger:hover:not(:disabled) {
  background: var(--danger-soft-bg);
  color: var(--danger);
}

.bulk-action-danger:hover:not(:disabled) .bulk-action-icon {
  color: var(--danger);
}

.bulk-flag-wrap {
  position: relative;
}

.bulk-flag-caret {
  color: var(--text-muted);
  margin-left: -2px;
}

.bulk-flag-popover {
  top: calc(100% + 6px);
  left: 0;
  padding: 6px;
  min-width: 184px;
  box-shadow: var(--shadow-lg);
}

.bulk-flag-swatches {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 2px;
}

.bulk-flag-swatch {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out), transform var(--dur-hover) var(--ease-out);
}

.bulk-flag-swatch:hover {
  background: var(--hover-surface);
  transform: scale(1.08);
}

.bulk-flag-clear {
  display: block;
  width: 100%;
  margin-top: 5px;
  padding: 4px 8px;
  border: none;
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius-xs) var(--radius-xs);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-meta);
  text-align: center;
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out);
}

.bulk-flag-clear:hover {
  background: var(--hover-surface);
  color: var(--text);
}

.bulk-right {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}

.bulk-close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-meta);
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out);
}

.bulk-close-btn:hover {
  background: var(--hover-surface);
  color: var(--text);
}

.bulk-bar-enter-active,
.bulk-bar-leave-active {
  transition: opacity var(--dur-state) var(--ease-out), transform var(--dur-state) var(--ease-out);
}

.bulk-bar-enter-from,
.bulk-bar-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
