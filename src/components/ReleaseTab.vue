<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { confirm } from '@tauri-apps/plugin-dialog'
import type { ReleaseInfo } from '../api/releases'
import {
  batchDeleteReleases,
  batchSetReleaseFlag,
  batchUpdateReleaseStatus,
  getReleaseDetail,
  setNotificationState,
} from '../api/releases'
import { isReadStatus, isUnreadStatus, filterReleaseIndices, type AutoReadDetailMode } from '../utils'
import ReleaseAggregatedList from './ReleaseAggregatedList.vue'
import ReleaseCalendar from './ReleaseCalendar.vue'
import ReleaseDateDetail from './ReleaseDateDetail.vue'
import ReleaseDetailModal from './ReleaseDetailModal.vue'
import ReleaseSearchBar from './ReleaseSearchBar.vue'
import ReleaseSimpleList from './ReleaseSimpleList.vue'
import type { ReleaseFlagFilter, ReleaseImportanceFilter, ReleaseSourceFilter, ReleaseStatusFilter, ReleaseVersionFilter, ViewMode } from './releaseTypes'
import { FLAG_ITEMS, flagColorByIndex, releaseFlagged } from '../utils/releaseFlag'
import { loadReleaseViewState, saveReleaseViewState } from '../utils/releaseViewStorage'
import { clampCalendarMonth, toDateKey } from '../utils/dateKey'
import { isEditableTarget } from '../utils/dom'
import { useBulkSelection } from '../composables/useBulkSelection'
import { hasActiveOverlay } from '../composables/contextMenuBus'
import { useReleaseDeepSearch } from '../composables/useReleaseDeepSearch'
import { track } from '../composables/useUsageTracking'
import { ShowImportanceKey, AutoReadDetailKey, ShowToastKey } from '../injection-keys'
import { t } from '../i18n'
import ErrorBlock from './common/ErrorBlock.vue'

// 通知定位（App.vue focus-release 事件）下钻到单条 release：
// - focusTarget：目标 release id（App 在收到通知点击时设置，供本组件消费）；
// - focusToken：递增令牌。App 与 release 列表的数据刷新（轮询/标记/删除…）解耦，
//   本组件在“可能使列表就绪/内容变化”的时机统一检查 token 是否变化并消费（只消费最新一次）。
// - focus-consumed：定位成功（目标已展示、滚动、高亮）后置位，交由 App 清空目标。
// - focus-not-found：目标在本列表（重置筛选后）找不到；由 App 判定后给出 Toast（见 App.vue）。
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
  /** 通知定位目标 release id（App 在 focus-release 事件时设置）。 */
  focusTarget?: number | null
  /** 定位令牌：每次目标变化时递增；本组件据此决定是否消费。 */
  focusToken?: number
  /** 目录加载中（App 侧状态）：传给列表，仅在列表为空时渲染骨架。 */
  loading?: boolean
  /** 目录加载失败信息：列表为空时用错误块代替空态，把「拉取失败」与「暂无版本」分开。 */
  loadError?: string
  /** 当前 tab 是否可见（App 用 v-show 切 tab，组件常驻挂载；j/k 快捷键只在可见时响应）。 */
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
  /** 目标已定位并展示。 */
  'focus-consumed': []
  /** 目标在本列表（重置筛选后）仍找不到（已删除）。 */
  'focus-not-found': []
  /** 目录加载失败后的重试出口（由 App 重新拉取）。 */
  reload: []
}>()

const savedViewState = loadReleaseViewState()
const viewMode = ref<ViewMode>(savedViewState.viewMode)
// 「显示重要度」开关（App.vue provide）：关闭时重要度筛选不参与过滤，但**不清值**——
// 持久化后清值等于永久丢掉用户的选择，开关重新打开即恢复（过滤处统一按 showImportance 短路）
const showImportance = inject(ShowImportanceKey, ref(false))
// 「打开详情自动标为已读」档位（设置页常规，App provide）：off / open / navigate
const autoReadDetail = inject(AutoReadDetailKey, ref<AutoReadDetailMode>('off'))
const showToast = inject(ShowToastKey, () => {})
const importanceFilter = ref<ReleaseImportanceFilter>(savedViewState.importanceFilter)
const sourceFilter = ref<ReleaseSourceFilter>(savedViewState.sourceFilter)
const flagFilter = ref<ReleaseFlagFilter>(savedViewState.flagFilter)
const versionFilter = ref<ReleaseVersionFilter>(savedViewState.versionFilter)
const selectedDate = ref<string | null>(null)
const calendarYear = ref(new Date().getFullYear())
const calendarMonth = ref(new Date().getMonth() + 1)
const simpleList = ref<ReleaseSimpleListHandle | null>(null)
const aggregatedList = ref<AggregatedListInstance | null>(null)
const searchBar = ref<InstanceType<typeof ReleaseSearchBar> | null>(null)

// ── 通知定位消费 ──────────────
// 只消费**最新一次**目标：lastConsumedToken 记录已成功处理的 token，重复触发不会重复滚动/高亮。
let lastConsumedToken = -1

// 等待一帧让重置后的筛选/视图渲染、虚拟列表可视行挂载完成。
function waitForListReady(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

async function doFocusTarget(targetId: number) {
  const token = props.focusToken
  const release = props.releases.find(r => r.id === targetId)
  if (!release) {
    // 目标不在当前数据中：若列表已有数据（加载完成过）说明目标确实不存在（已删除），
    // 上报 App 提示；若列表仍为空（冷启动尚未加载完成）则不消费，等数据 watch 重试。
    if (props.releases.length > 0 && token > lastConsumedToken) {
      lastConsumedToken = token
      emit('focus-not-found')
    }
    return
  }
  // 重置全部筛选并切到简单视图：目标必须不被任何过滤隐藏，且视图支持按 id 滚动定位。
  // 这次重置是定位的手段而非用户偏好，期间禁止把状态写回存储，否则点一次通知
  // 就会永久抹掉用户保存的视图/筛选。
  suspendViewStatePersist = true
  try {
    releaseSearch.value = ''
    statusFilter.value = 'all'
    importanceFilter.value = 'all'
    sourceFilter.value = 'all'
    flagFilter.value = 'all'
    versionFilter.value = 'all'
    viewMode.value = 'simple'
    selectedDate.value = null
    await waitForListReady()
  } finally {
    suspendViewStatePersist = false
  }
  if (props.focusTarget !== targetId || props.focusToken !== token) {
    // 期间 App 已清空或来了更新的目标：丢弃本次（避免旧目标覆盖新目标的高亮）
    return
  }
  // 简单视图列表可能因虚拟化只渲染可视区：先滚到目标再置高亮（等待行挂载）。
  // 首次可能失败：目标存在于 props.releases（未过滤），但 SimpleList 用的是
  // filteredReleases，视图刚从 aggregated/calendar 切回或列表重建时其 ref 尚未就绪。
  // 因此再等一帧重试一次；仍失败则一律按「定位不到」上报——任何情况下都不留静默失败。
  let handled = simpleList.value?.focusReleaseId?.(targetId) ?? false
  if (!handled) {
    await nextTick()
    if (props.focusTarget !== targetId || props.focusToken !== token) return
    handled = simpleList.value?.focusReleaseId?.(targetId) ?? false
  }
  lastConsumedToken = token
  if (handled) {
    emit('focus-consumed')
  } else {
    emit('focus-not-found')
  }
}

// 定位时机：目标 token 变化 / 数据（releases 引用）变化都可能使“目标可定位”。
function consumeFocusIfPending() {
  const target = props.focusTarget
  if (target === null || props.focusToken <= lastConsumedToken) return
  void doFocusTarget(target)
}

watch(() => props.focusToken, consumeFocusIfPending, { flush: 'post' })
watch(() => props.releases, consumeFocusIfPending, { flush: 'post' })

const releaseSearch = computed({
  get: () => props.search,
  set: (value: string) => emit('update:search', value),
})

const statusFilter = computed({
  get: () => props.statusFilter,
  set: (value: ReleaseStatusFilter) => emit('update:statusFilter', value),
})

let suspendViewStatePersist = false
watch([viewMode, statusFilter, importanceFilter, sourceFilter, flagFilter, versionFilter], () => {
  if (suspendViewStatePersist) return
  saveReleaseViewState({
    viewMode: viewMode.value,
    statusFilter: statusFilter.value,
    importanceFilter: importanceFilter.value,
    sourceFilter: sourceFilter.value,
    flagFilter: flagFilter.value,
    versionFilter: versionFilter.value,
  })
})

const hasActiveFilter = computed(() => {
  return releaseSearch.value.trim() !== '' || statusFilter.value !== 'all' || (showImportance.value && importanceFilter.value !== 'all') || sourceFilter.value !== 'all' || flagFilter.value !== 'all' || versionFilter.value !== 'all'
})

// 目录拉取失败只应在列表为空时占据空态位置：已有数据时旧内容还能读，错误改由 Toast 提示
const showLoadError = computed(() => props.loadError !== '' && props.releases.length === 0)

// 空态的两个出口：搜索无结果清搜索，筛选无结果清筛选（菜单里的 chip 未展开时用户看不到全部条件）
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

const filteredReleases = computed(() => {
  let list = props.releases

  const q = releaseSearch.value.trim()
  if (q) {
    // 深度搜索关闭时不用索引：正文索引在会话内驻留（重复开关不重拉），
    // 是否参与命中必须由 deepSearch 显式决定，不能靠 bodyIndex 是否为空。
    const picked = filterReleaseIndices(list, q, deepSearch.value ? bodyIndex.value : null)
    list = picked.map(i => list[i])
  }

  if (statusFilter.value === 'unread') {
    list = list.filter(release => isUnreadStatus(release.notification_status, release.snooze_until))
  } else if (statusFilter.value === 'read') {
    list = list.filter(release => isReadStatus(release.notification_status))
  }

  if (showImportance.value && importanceFilter.value !== 'all') {
    list = list.filter(release => release.ai_importance === importanceFilter.value)
  }

  if (sourceFilter.value !== 'all') {
    list = list.filter(release => release.source_type === sourceFilter.value)
  }

  if (flagFilter.value === 'flagged') {
    list = list.filter(releaseFlagged)
  } else if (flagFilter.value === 'unflagged') {
    list = list.filter(release => !releaseFlagged(release))
  } else if (flagFilter.value !== 'all') {
    list = list.filter(release => release.flag === flagFilter.value)
  }

  if (versionFilter.value !== 'all') {
    if (versionFilter.value === 'prerelease') {
      list = list.filter(release => release.prerelease)
    } else {
      list = list.filter(release => release.version_bump === versionFilter.value)
    }
  }

  return list
})

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

async function handleSearchEnter() {
  if (viewMode.value === 'aggregated') {
    // 搜索框 Enter 时已 flush，但新词要经宿主回传（props 更新）才可见：
    // 等一帧再展开，否则 expandAll 展开的是输入前的全量分组。
    await nextTick()
    aggregatedList.value?.expandAll()
    return
  }
  if (viewMode.value === 'simple') {
    await nextTick()
    // 回车选中首条后把焦点移出搜索框，j/k/Enter 列表导航立即可用
    if (simpleList.value?.selectFirst()) {
      const active = document.activeElement
      if (active instanceof HTMLInputElement) active.blur()
    }
  }
}

function backToCalendar() {
  selectedDate.value = null
}

function shiftMonth(delta: number) {
  const next = clampCalendarMonth(calendarYear.value, calendarMonth.value + delta)
  calendarYear.value = next.year
  calendarMonth.value = next.month
}

function prevMonth() {
  shiftMonth(-1)
}

function nextMonth() {
  shiftMonth(1)
}

function jumpCalendar(year: number, month: number) {
  const next = clampCalendarMonth(year, month)
  calendarYear.value = next.year
  calendarMonth.value = next.month
  selectedDate.value = null
}

watch(viewMode, () => {
  selectedDate.value = null
})

// ========== 版本详情弹窗 ==========
// 弹窗导航序列由打开时的视图上下文决定（简单=全局时间序，聚合=同仓库，日历=当日），
// 各列表组件通过 open-detail 事件携带。只存 id 序列，release 对象实时从 props.releases
// 查找，保证列表刷新（如翻译完成）后弹窗内容同步更新。
const detailReleaseId = ref<number | null>(null)
const detailSequenceIds = ref<number[]>([])
// 目录里的正文是预览投影，打开详情时按 id 取全文。取到前用列表项（预览）渲染，
// 因此 ReleaseDetailModal 无需感知「预览 / 全文」的区别（替换窗口是一次本地 IPC）。
const detailFull = shallowRef<ReleaseInfo | null>(null)
let detailToken = 0

const detailIndex = computed(() => detailSequenceIds.value.indexOf(detailReleaseId.value ?? -1))
const detailRelease = computed(() => {
  const id = detailReleaseId.value
  if (id === null) return null
  const item = props.releases.find(r => r.id === id)
  if (!item) return null
  const full = detailFull.value
  if (!full || full.id !== id) return item
  return {
    ...item,
    body: full.body ?? item.body,
    body_translated: full.body_translated ?? item.body_translated,
  }
})

async function loadReleaseDetail(id: number) {
  const token = ++detailToken
  try {
    const full = await getReleaseDetail(id)
    // 竞态防护：期间已切换目标 / 关闭弹窗则丢弃
    if (token !== detailToken || detailReleaseId.value !== id) return
    detailFull.value = full
  } catch {
    // 取全文失败不打断阅读：保留目录里的预览
  }
}

function openReleaseDetail(release: ReleaseInfo, sequence: ReleaseInfo[]) {
  detailSequenceIds.value = sequence.map(r => r.id)
  detailReleaseId.value = release.id
  if (autoReadDetail.value !== 'off') autoMarkRead(release)
}

function closeReleaseDetail() {
  detailReleaseId.value = null
  detailSequenceIds.value = []
}

// 目标变化（打开 / 逐条导航 / 关闭）即取全文：先清掉上一条的全文，避免串内容
watch(detailReleaseId, (id) => {
  detailFull.value = null
  if (id !== null) void loadReleaseDetail(id)
})

// 目录刷新（轮询 / 翻译落库 / 标记）后在打开中的条目上重取全文：翻译完成后同步内容
watch(() => props.releases, () => {
  if (detailReleaseId.value !== null) void loadReleaseDetail(detailReleaseId.value)
})

function navigateReleaseDetail(delta: number) {
  track(delta < 0 ? 'release.detail_prev' : 'release.detail_next')
  const nextId = detailSequenceIds.value[detailIndex.value + delta]
  if (nextId === undefined) return
  detailReleaseId.value = nextId
  if (autoReadDetail.value === 'navigate') {
    const next = props.releases.find(r => r.id === nextId)
    if (next) autoMarkRead(next)
  }
}

/** 打开详情自动标记已读：只对未读条目生效（已忽略/已读的不改写状态），
 *  失败静默——自动行为不该打断阅读，取消未读留待用户手动处理。
 *  在途去重：快速来回翻页时列表尚未刷新（条目仍显示 pending），
 *  不去重会对同一条重复发 IPC、重复写操作日志。 */
const autoMarkingIds = new Set<number>()

function autoMarkRead(release: ReleaseInfo) {
  if (!isUnreadStatus(release.notification_status, release.snooze_until)) return
  if (autoMarkingIds.has(release.id)) return
  autoMarkingIds.add(release.id)
  setNotificationState(release.id, 'clicked')
    .then(() => emit('update'))
    .catch(() => {})
    .finally(() => autoMarkingIds.delete(release.id))
}

// 宿主（App 的 Ctrl+F）调用：弹出中的详情会盖住搜索栏，先关掉再聚焦才有可见反馈
function focusSearch() {
  closeReleaseDetail()
  void nextTick(() => searchBar.value?.focusInput())
}

defineExpose({ focusSearch })

// ── 简单视图键盘导航（j/k 移动、Enter 打开详情）──────────────
// tab 用 v-show 常驻挂载，必须等 active 才响应；输入控件/可编辑区域内的按键不拦截。

const BULK_FLAG_SWATCHES = FLAG_ITEMS

const bulkBusy = ref(false)
const flagPopoverOpen = ref(false)

const canSelect = computed(() => viewMode.value !== 'calendar' || selectedDate.value !== null)

const visibleSelectionReleases = computed<ReleaseInfo[]>(() => {
  if (viewMode.value === 'calendar') {
    if (selectedDate.value === null) return []
    return filteredReleases.value
      .filter(r => toDateKey(new Date(r.published_at)) === selectedDate.value)
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
    .filter(r => isUnreadStatus(r.notification_status, r.snooze_until))
    .map(r => r.id),
)

const visibleSelectionIds = computed(() => visibleSelectionReleases.value.map(r => r.id))

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
  onExit: () => {
    flagPopoverOpen.value = false
  },
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
    if (affected > 0) showToast(toastKey(affected))
    emit('update')
  } catch (e: unknown) {
    showToast(t(errPrefixKey) + (e instanceof Error ? e.message : String(e)))
  } finally {
    bulkBusy.value = false
  }
}

async function confirmAndMarkRead(ids: number[], confirmMsg: string, titleKey: string, trackEvent: string) {
  if (ids.length === 0 || bulkBusy.value) return
  const confirmed = await confirm(confirmMsg, { title: t(titleKey), kind: 'info' })
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
  const confirmed = await confirm(t('release.bulk_delete_confirm', String(ids.length)), {
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

function closeFlagPopover() {
  flagPopoverOpen.value = false
}

function onListKeydown(e: KeyboardEvent) {
  if (!props.active || e.isComposing || e.ctrlKey || e.altKey || e.metaKey) return
  if (detailReleaseId.value !== null || viewMode.value !== 'simple') return
  // 任意浮层（卡片封面灯箱、右键菜单、下拉面板）打开时让出快捷键，
  // 否则灯箱里按 j/k 会移动背后选中项、按 Enter 会在灯箱后弹出详情
  if (hasActiveOverlay()) return
  if (isEditableTarget(e.target)) return
  if (e.key === 'j') {
    e.preventDefault()
    simpleList.value?.moveSelection(1)
  } else if (e.key === 'k') {
    e.preventDefault()
    simpleList.value?.moveSelection(-1)
  } else if (e.key === 'Enter') {
    // 焦点在按钮/链接/菜单上时让原生点击生效，不抢
    const target = e.target instanceof HTMLElement ? e.target : null
    if (target?.closest('button, a, [role="menu"], [role="menuitem"]')) return
    simpleList.value?.openSelected()
  }
}

onMounted(() => {
  window.addEventListener('keydown', onListKeydown)
  document.addEventListener('click', closeFlagPopover)
})
onUnmounted(() => {
  window.removeEventListener('keydown', onListKeydown)
  document.removeEventListener('click', closeFlagPopover)
})

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
              <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#check-icon"/></svg>
              <span>{{ t('release.bulk_read') }}</span>
            </button>
            <button
              type="button"
              class="bulk-action-btn"
              :disabled="selectedCount === 0 || bulkBusy"
              @click="handleBulkStatus('ignored')"
            >
              <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#ignore-icon"/></svg>
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
                <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#flag-tag-icon"/></svg>
                <span>{{ t('release.bulk_flag') }}</span>
                <svg class="bulk-flag-caret" width="10" height="10" aria-hidden="true"><use href="/icons.svg#chevron-down-icon"/></svg>
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
                    <svg width="15" height="15" aria-hidden="true"><use href="/icons.svg#flag-tag-icon"/></svg>
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
                <svg class="bulk-action-icon" aria-hidden="true"><use href="/icons.svg#trash-icon"/></svg>
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
