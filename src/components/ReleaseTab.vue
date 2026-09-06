<script setup lang="ts">
import { computed, inject, ref, shallowRef, watch } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { isReadStatus, isUnreadStatus, filterReleaseIndices, buildBodyIndex } from '../utils'
import ReleaseAggregatedList from './ReleaseAggregatedList.vue'
import ReleaseCalendar from './ReleaseCalendar.vue'
import ReleaseDateDetail from './ReleaseDateDetail.vue'
import ReleaseDetailModal from './ReleaseDetailModal.vue'
import ReleaseSearchBar from './ReleaseSearchBar.vue'
import ReleaseSimpleList from './ReleaseSimpleList.vue'
import type { ReleaseFlagFilter, ReleaseImportanceFilter, ReleaseSourceFilter, ReleaseStatusFilter, ReleaseVersionFilter, ViewMode } from './releaseTypes'
import { releaseFlagged } from '../utils/releaseFlag'
import { track } from '../composables/useUsageTracking'
import { ShowImportanceKey } from '../injection-keys'

type AggregatedListInstance = InstanceType<typeof ReleaseAggregatedList> & {
  expandAll: () => void
}

const props = defineProps<{
  releases: ReleaseInfo[]
  search?: string
  statusFilter?: ReleaseStatusFilter
}>()

const emit = defineEmits<{
  update: []
  'update:search': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
}>()

const viewMode = ref<ViewMode>('simple')
const importanceFilter = ref<ReleaseImportanceFilter>('all')
// 「显示重要度」开关（App.vue provide）：关闭时不参与过滤，并清掉残留的重要度筛选
const showImportance = inject(ShowImportanceKey, ref(true))
watch(showImportance, (visible) => {
  if (!visible) importanceFilter.value = 'all'
})
const sourceFilter = ref<ReleaseSourceFilter>('all')
const flagFilter = ref<ReleaseFlagFilter>('all')
const versionFilter = ref<ReleaseVersionFilter>('all')
const selectedDate = ref<string | null>(null)
const calendarYear = ref(new Date().getFullYear())
const calendarMonth = ref(new Date().getMonth() + 1)
const aggregatedList = ref<AggregatedListInstance | null>(null)

const releaseSearch = computed({
  get: () => props.search ?? '',
  set: (value: string) => emit('update:search', value),
})

const statusFilter = computed({
  get: () => props.statusFilter ?? 'all',
  set: (value: ReleaseStatusFilter) => emit('update:statusFilter', value),
})

const hasActiveFilter = computed(() => {
  return releaseSearch.value.trim() !== '' || statusFilter.value !== 'all' || (showImportance.value && importanceFilter.value !== 'all') || sourceFilter.value !== 'all' || flagFilter.value !== 'all' || versionFilter.value !== 'all'
})

const filteredReleases = computed(() => {
  let list = props.releases

  const q = releaseSearch.value.trim()
  if (q) {
    const picked = filterReleaseIndices(list, q, bodyIndex.value)
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

// ── 深度搜索（Tier2：GitHub / HF 正文与译文全文）──────────────
// 常规搜索只走 Tier1（元数据 + AI 摘要 + 视频源简介）；深度搜索临时构建
// Tier2 索引，搜完/关闭即释放（Tier2 可达几十 MB，见 docs §1.5）。
const deepSearch = ref(false)              // 是否处于深度搜索态
const bodyIndex = shallowRef<string[][] | null>(null)
const deepSearching = ref(false)           // loading 态

// 关闭深度搜索时释放 Tier2（关键：不释放则常驻几十 MB）
watch(deepSearch, (on) => {
  if (!on) bodyIndex.value = null
})
// releases 引用变化时旧索引失效（轮询完成 / 标记已读等都会整体替换 releases.value）。
// 若此时处于深度搜索态，必须就地重建：否则 deepSearch 仍为 true、按钮仍高亮，
// 过滤却只剩 Tier1，body 命中结果静默消失。重建成本同 runDeepSearch（约 100ms 量级），
// 且只在深度搜索会话内发生，可接受。
watch(() => props.releases, () => {
  bodyIndex.value = null
  if (deepSearch.value && releaseSearch.value.trim()) {
    bodyIndex.value = buildBodyIndex(props.releases)
  }
})
// 搜索词被清空时自动退出深度搜索态并释放索引
watch(releaseSearch, (q) => {
  if (!q.trim()) {
    deepSearch.value = false
    bodyIndex.value = null
  }
})

async function runDeepSearch() {
  if (!releaseSearch.value.trim()) return
  deepSearching.value = true
  // 让出一帧，保证 loading 态能渲染出来（20× 下构建约 100ms）
  await new Promise(r => requestAnimationFrame(() => r(null)))
  // 竞态防护：等待期间用户已关闭深度搜索（或清空搜索词），丢弃本次构建
  if (!deepSearch.value || !releaseSearch.value.trim()) {
    deepSearching.value = false
    return
  }
  bodyIndex.value = buildBodyIndex(props.releases)
  deepSearching.value = false
}

function onDeepSearchToggle(on: boolean) {
  deepSearch.value = on
  if (on) void runDeepSearch()
}

function enableDeepSearch() {
  deepSearch.value = true
  void runDeepSearch()
}

function handleSearchEnter() {
  if (viewMode.value === 'aggregated') aggregatedList.value?.expandAll()
}

function backToCalendar() {
  selectedDate.value = null
}

function prevMonth() {
  // 下限保护：不允许早于 2010-01，与 nextMonth 的上限保护对称，避免远古日期渲染异常
  const MIN_YEAR = 2010
  if (calendarYear.value <= MIN_YEAR && calendarMonth.value === 1) return
  if (calendarMonth.value === 1) {
    calendarMonth.value = 12
    calendarYear.value--
  } else {
    calendarMonth.value--
  }
}

function nextMonth() {
  const current = new Date()
  const currentYear = current.getFullYear()
  const currentMonth = current.getMonth() + 1
  const nextYear = calendarMonth.value === 12 ? calendarYear.value + 1 : calendarYear.value
  const nextMonthValue = calendarMonth.value === 12 ? 1 : calendarMonth.value + 1
  if (nextYear > currentYear || (nextYear === currentYear && nextMonthValue > currentMonth)) return

  calendarYear.value = nextYear
  calendarMonth.value = nextMonthValue
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

const detailIndex = computed(() => detailSequenceIds.value.indexOf(detailReleaseId.value ?? -1))
const detailRelease = computed(() => {
  if (detailReleaseId.value === null) return null
  return props.releases.find(r => r.id === detailReleaseId.value) ?? null
})

function openReleaseDetail(release: ReleaseInfo, sequence: ReleaseInfo[]) {
  detailReleaseId.value = release.id
  detailSequenceIds.value = sequence.map(r => r.id)
}

function closeReleaseDetail() {
  detailReleaseId.value = null
  detailSequenceIds.value = []
}

function navigateReleaseDetail(delta: number) {
  track(delta < 0 ? 'release.detail_prev' : 'release.detail_next')
  const nextId = detailSequenceIds.value[detailIndex.value + delta]
  if (nextId !== undefined) detailReleaseId.value = nextId
}

</script>

<template>
  <section class="tab-content">
    <ReleaseSearchBar
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
      @update:deep-search="onDeepSearchToggle"
      @search-enter="handleSearchEnter"
    />

    <ReleaseSimpleList
      v-if="viewMode === 'simple'"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      :has-search-query="releaseSearch.trim() !== ''"
      :deep-search="deepSearch"
      @enable-deep="enableDeepSearch"
      @update="emit('update')"
      @open-detail="openReleaseDetail"
    />

    <ReleaseAggregatedList
      v-else-if="viewMode === 'aggregated'"
      ref="aggregatedList"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      @update="emit('update')"
      @open-detail="openReleaseDetail"
    />

    <template v-else-if="viewMode === 'calendar'">
      <ReleaseDateDetail
        v-if="selectedDate !== null"
        :selected-date="selectedDate"
        :releases="filteredReleases"
        @back="backToCalendar"
        @update="emit('update')"
        @open-detail="openReleaseDetail"
      />
      <ReleaseCalendar
        v-else
        :releases="filteredReleases"
        :year="calendarYear"
        :month="calendarMonth"
        @prev-month="prevMonth"
        @next-month="nextMonth"
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
      @close="closeReleaseDetail"
      @navigate="navigateReleaseDetail"
      @update="emit('update')"
    />

  </section>
</template>
