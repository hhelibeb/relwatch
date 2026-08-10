<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { isReadStatus, isUnreadStatus, releaseMatchesSearch } from '../utils'
import ReleaseAggregatedList from './ReleaseAggregatedList.vue'
import ReleaseCalendar from './ReleaseCalendar.vue'
import ReleaseDateDetail from './ReleaseDateDetail.vue'
import ReleaseDetailModal from './ReleaseDetailModal.vue'
import ReleaseSimpleList from './ReleaseSimpleList.vue'
import ReleaseToolbar from './ReleaseToolbar.vue'
import type { ReleaseImportanceFilter, ReleaseStatusFilter, ViewMode } from './releaseTypes'
import { track } from '../composables/useUsageTracking'

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
  return releaseSearch.value.trim() !== '' || statusFilter.value !== 'all' || importanceFilter.value !== 'all'
})

const filteredReleases = computed(() => {
  let list = props.releases

  const q = releaseSearch.value.trim().toLowerCase()
  if (q) list = list.filter(release => releaseMatchesSearch(release, q))

  if (statusFilter.value === 'unread') {
    list = list.filter(release => isUnreadStatus(release.notification_status, release.snooze_until))
  } else if (statusFilter.value === 'read') {
    list = list.filter(release => isReadStatus(release.notification_status))
  }

  if (importanceFilter.value !== 'all') {
    list = list.filter(release => release.ai_importance === importanceFilter.value)
  }

  return list
})

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
    <ReleaseToolbar
      v-model="releaseSearch"
      v-model:status-filter="statusFilter"
      v-model:importance-filter="importanceFilter"
      v-model:view-mode="viewMode"
      @search-enter="handleSearchEnter"
    />

    <ReleaseSimpleList
      v-if="viewMode === 'simple'"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
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
