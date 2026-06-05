<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { isReadStatus, isUnreadStatus, releaseMatchesSearch } from '../utils'
import ReleaseAggregatedList from './ReleaseAggregatedList.vue'
import ReleaseCalendar from './ReleaseCalendar.vue'
import ReleaseDateDetail from './ReleaseDateDetail.vue'
import ReleaseSimpleList from './ReleaseSimpleList.vue'
import ReleaseToolbar from './ReleaseToolbar.vue'
import type { ReleaseImportanceFilter, ReleaseStatusFilter, ViewMode } from './releaseTypes'

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
    list = list.filter(release => isUnreadStatus(release.notification_status))
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
    />

    <ReleaseAggregatedList
      v-else-if="viewMode === 'aggregated'"
      ref="aggregatedList"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      @update="emit('update')"
    />

    <template v-else-if="viewMode === 'calendar'">
      <ReleaseDateDetail
        v-if="selectedDate !== null"
        :selected-date="selectedDate"
        :releases="filteredReleases"
        @back="backToCalendar"
        @update="emit('update')"
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
  </section>
</template>
