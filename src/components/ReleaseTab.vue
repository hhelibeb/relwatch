<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { type ReleaseInfo, openReleaseUrl } from '../api'
import { t, getLocale } from '../i18n'
import { formatDate, isReadStatus, isUnreadStatus, releaseMatchesSearch } from '../utils'
import ReleaseItem from './ReleaseItem.vue'
import ReleaseSearchBar from './ReleaseSearchBar.vue'

const props = defineProps<{ releases: ReleaseInfo[]; search?: string }>()
const emit = defineEmits<{ update: []; 'update:search': [value: string] }>()

type ViewMode = 'simple' | 'aggregated' | 'calendar'
const viewMode = ref<ViewMode>('simple')
const releaseSearch = computed({
  get: () => props.search ?? '',
  set: (value: string) => emit('update:search', value),
})
const expandedRepos = ref<Set<string>>(new Set())
const selectedDate = ref<string | null>(null)
const calendarYear = ref(new Date().getFullYear())
const calendarMonth = ref(new Date().getMonth() + 1)
const tooltip = ref<{ x: number; y: number; date: string; releases: ReleaseInfo[] } | null>(null)

const statusFilter = ref<'all' | 'unread' | 'read'>('all')
const importanceFilter = ref<'all' | '大' | '中' | '小'>('all')

// ========== 筛选 ==========
const filteredReleases = computed(() => {
  let list = props.releases

  const q = releaseSearch.value.trim().toLowerCase()
  if (q) list = list.filter(r => releaseMatchesSearch(r, q))

  if (statusFilter.value === 'unread') {
    list = list.filter(r => isUnreadStatus(r.notification_status))
  } else if (statusFilter.value === 'read') {
    list = list.filter(r => isReadStatus(r.notification_status))
  }

  if (importanceFilter.value !== 'all') {
    list = list.filter(r => r.ai_importance === importanceFilter.value)
  }

  return list
})

const sortedReleases = computed(() => {
  return [...filteredReleases.value].sort(
    (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime()
  )
})

// ========== 聚合视图 ==========
const repoGroups = computed(() => {
  const map = new Map<string, ReleaseInfo[]>()
  for (const r of filteredReleases.value) {
    const key = `${r.owner}/${r.repo}`
    if (!map.has(key)) map.set(key, [])
    map.get(key)!.push(r)
  }
  const groups: { key: string; releases: ReleaseInfo[] }[] = []
  for (const [key, releases] of map) {
    releases.sort((a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime())
    groups.push({ key, releases })
  }
  groups.sort((a, b) => new Date(b.releases[0].published_at).getTime() - new Date(a.releases[0].published_at).getTime())
  return groups
})

function toggleRepo(key: string) {
  const next = new Set(expandedRepos.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  expandedRepos.value = next
}

function expandAllSearchResults() {
  const next = new Set<string>()
  for (const g of repoGroups.value) {
    next.add(g.key)
  }
  expandedRepos.value = next
}

const allExpanded = computed(() => {
  if (repoGroups.value.length === 0) return false
  return repoGroups.value.every(g => expandedRepos.value.has(g.key))
})

function toggleAllRepos() {
  if (allExpanded.value) {
    expandedRepos.value = new Set()
  } else {
    expandAllSearchResults()
  }
}

watch(() => repoGroups.value, () => {
  expandedRepos.value = new Set()
}, { immediate: true })

// ========== 日历视图 ==========
function toDateKey(date: Date): string {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, '0')
  const d = String(date.getDate()).padStart(2, '0')
  return `${y}-${m}-${d}`
}

function parseDateKey(key: string): Date {
  const [y, m, d] = key.split('-').map(Number)
  return new Date(y, m - 1, d)
}

const calendarMap = computed(() => {
  const map = new Map<string, ReleaseInfo[]>()
  for (const r of filteredReleases.value) {
    const key = toDateKey(new Date(r.published_at))
    if (!map.has(key)) map.set(key, [])
    map.get(key)!.push(r)
  }
  return map
})

const todayKey = toDateKey(new Date())

interface CalendarCell {
  date: number
  key: string
  count: number
  isCurrentMonth: boolean
  isToday: boolean
  releases: ReleaseInfo[]
}

const weekDayHeaders = computed(() => {
  const locale = getLocale()
  const isZh = locale === 'zh-CN'
  if (isZh) return ['日', '一', '二', '三', '四', '五', '六']
  return ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
})

const monthLabel = computed(() => {
  const d = new Date(calendarYear.value, calendarMonth.value - 1, 1)
  const locale = getLocale()
  return d.toLocaleDateString(locale, { year: 'numeric', month: 'long' })
})

const monthGrid = computed(() => {
  const year = calendarYear.value
  const month = calendarMonth.value
  const firstDay = new Date(year, month - 1, 1)
  const lastDay = new Date(year, month, 0)
  const totalDays = lastDay.getDate()
  const startDow = firstDay.getDay()

  const cells: CalendarCell[] = []

  const prevMonthLastDay = new Date(year, month - 1, 0).getDate()
  for (let i = startDow - 1; i >= 0; i--) {
    const d = new Date(year, month - 2, prevMonthLastDay - i)
    const key = toDateKey(d)
    const entry = calendarMap.value.get(key)
    cells.push({
      date: prevMonthLastDay - i,
      key,
      count: entry ? entry.length : 0,
      isCurrentMonth: false,
      isToday: key === todayKey,
      releases: entry || [],
    })
  }

  for (let day = 1; day <= totalDays; day++) {
    const d = new Date(year, month - 1, day)
    const key = toDateKey(d)
    const entry = calendarMap.value.get(key)
    cells.push({
      date: day,
      key,
      count: entry ? entry.length : 0,
      isCurrentMonth: true,
      isToday: key === todayKey,
      releases: entry || [],
    })
  }

  const remaining = 7 - (cells.length % 7)
  if (remaining < 7 || cells.length < 35) {
    const fill = remaining < 7 ? remaining + (cells.length < 35 ? 7 : 0) : 0
    for (let i = 1; i <= fill; i++) {
      const d = new Date(year, month, i)
      const key = toDateKey(d)
      const entry = calendarMap.value.get(key)
      cells.push({
        date: i,
        key,
        count: entry ? entry.length : 0,
        isCurrentMonth: false,
        isToday: key === todayKey,
        releases: entry || [],
      })
    }
  }

  return cells
})

const now = new Date()
const currentYear = now.getFullYear()
const currentMonth = now.getMonth() + 1

const isNextDisabled = computed(() => {
  const nextY = calendarMonth.value === 12 ? calendarYear.value + 1 : calendarYear.value
  const nextM = calendarMonth.value === 12 ? 1 : calendarMonth.value + 1
  return nextY > currentYear || (nextY === currentYear && nextM > currentMonth)
})

function prevMonth() {
  if (calendarMonth.value === 1) {
    calendarMonth.value = 12
    calendarYear.value--
  } else {
    calendarMonth.value--
  }
}

function nextMonth() {
  let y = calendarMonth.value === 12 ? calendarYear.value + 1 : calendarYear.value
  let m = calendarMonth.value === 12 ? 1 : calendarMonth.value + 1
  if (y > currentYear || (y === currentYear && m > currentMonth)) return
  if (calendarMonth.value === 12) {
    calendarMonth.value = 1
    calendarYear.value++
  } else {
    calendarMonth.value++
  }
}

function countClass(count: number): string {
  if (count >= 4) return 'calendar-cell-count-4'
  if (count >= 3) return 'calendar-cell-count-3'
  if (count >= 2) return 'calendar-cell-count-2'
  if (count >= 1) return 'calendar-cell-count-1'
  return ''
}

function handleCellHover(e: MouseEvent, cell: CalendarCell) {
  if (!cell.isCurrentMonth || cell.count === 0) {
    tooltip.value = null
    return
  }
  tooltip.value = { x: e.clientX + 12, y: e.clientY + 12, date: cell.key, releases: cell.releases }
}

function handleCellLeave() {
  tooltip.value = null
}

function handleCellClick(cell: CalendarCell) {
  if (!cell.isCurrentMonth || cell.count === 0) return
  tooltip.value = null
  selectedDate.value = cell.key
}

const dateDetailReleases = computed(() => {
  if (!selectedDate.value) return []
  return [...(calendarMap.value.get(selectedDate.value) || [])].sort(
    (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime()
  )
})

const dateDetailTitle = computed(() => {
  if (!selectedDate.value) return ''
  const d = parseDateKey(selectedDate.value)
  const locale = getLocale()
  return d.toLocaleDateString(locale, { year: 'numeric', month: 'long', day: 'numeric' })
})

function backToCalendar() {
  selectedDate.value = null
}

watch(viewMode, () => {
  selectedDate.value = null
})

// ========== Repo 级别的右键菜单（聚合视图的 repo header 专用） ==========
const repoContextMenu = ref<{ x: number; y: number; url: string } | null>(null)

function closeRepoContextMenu() {
  repoContextMenu.value = null
}
onMounted(() => document.addEventListener('click', closeRepoContextMenu))
onUnmounted(() => document.removeEventListener('click', closeRepoContextMenu))

function handleRepoContextMenu(e: MouseEvent, url: string) {
  repoContextMenu.value = { x: e.clientX, y: e.clientY, url }
}

async function handleRepoCopyLink() {
  try { await navigator.clipboard.writeText(repoContextMenu.value!.url) } catch { /* ignore */ }
  closeRepoContextMenu()
}

function handleRepoOpenLink() {
  openReleaseUrl(repoContextMenu.value!.url)
  closeRepoContextMenu()
}

function handleOpenUrl(url: string) {
  openReleaseUrl(url)
}
</script>

<template>
  <section class="tab-content">
    <ReleaseSearchBar
      v-model="releaseSearch"
      v-model:statusFilter="statusFilter"
      v-model:importanceFilter="importanceFilter"
      v-model:viewMode="viewMode"
      :showSearch="viewMode !== 'calendar' || selectedDate !== null"
      @searchEnter="viewMode !== 'simple' ? expandAllSearchResults() : undefined"
    />

    <!-- ============ 简单视图 ============ -->
    <template v-if="viewMode === 'simple'">
      <div class="release-list">
        <div v-if="sortedReleases.length === 0" class="empty">
          {{ releaseSearch || statusFilter !== 'all' || importanceFilter !== 'all' ? t('release.no_match') : t('release.empty') }}
        </div>
        <ReleaseItem
          v-for="release in sortedReleases"
          :key="release.id"
          :release="release"
          @update="emit('update')"
        />
      </div>
    </template>

    <!-- ============ 聚合视图 ============ -->
    <template v-if="viewMode === 'aggregated'">
      <div v-if="repoGroups.length === 0" class="empty">
        {{ releaseSearch || statusFilter !== 'all' || importanceFilter !== 'all' ? t('release.no_match') : t('release.empty') }}
      </div>
      <div v-else class="repo-toolbar">
        <button class="btn-sm" @click="toggleAllRepos">
          <svg class="toggle-all-icon" :class="{ 'icon-collapse': allExpanded }"><use href="/icons.svg#chevron-down-icon"/></svg>
          {{ allExpanded ? t('release.collapse_all') : t('release.expand_all') }}
        </button>
      </div>
      <div v-for="group in repoGroups" :key="group.key" class="repo-group">
        <div class="repo-group-header" @click="toggleRepo(group.key)">
          <button class="repo-group-toggle" :class="{ expanded: expandedRepos.has(group.key) }" @click.stop="toggleRepo(group.key)">
            <svg><use href="/icons.svg#chevron-down-icon"/></svg>
          </button>
          <span class="repo-name">{{ group.key }}</span>
          <span class="repo-latest-tag">{{ group.releases[0].tag_name }}</span>
          <button class="btn-icon-link" @click.stop="handleOpenUrl(group.releases[0].html_url)" @contextmenu.prevent.stop="handleRepoContextMenu($event, group.releases[0].html_url)" :title="t('release.open_link')">
            <svg><use href="/icons.svg#link-icon"/></svg>
          </button>
          <span class="repo-latest-date">{{ formatDate(group.releases[0].published_at) }}</span>
          <span class="repo-meta">{{ t('release.versions', [group.releases.length]) }}</span>
        </div>
        <div v-if="expandedRepos.has(group.key)" class="repo-group-body">
          <ReleaseItem
            v-for="release in group.releases"
            :key="release.id"
            :release="release"
            @update="emit('update')"
          />
        </div>
      </div>
    </template>

    <!-- ============ 日历视图 ============ -->
    <template v-if="viewMode === 'calendar'">
      <!-- 钻取到某天的版本列表 -->
      <template v-if="selectedDate !== null">
        <button class="calendar-back" @click="backToCalendar">
          <svg><use href="/icons.svg#chevron-left-icon"/></svg>
          {{ t('release.back_calendar') }}
        </button>
        <ReleaseSearchBar
          v-model="releaseSearch"
          v-model:statusFilter="statusFilter"
          v-model:importanceFilter="importanceFilter"
          v-model:viewMode="viewMode"
          :showSearch="true"
          @searchEnter="expandAllSearchResults"
        />
        <div class="date-detail-title">{{ dateDetailTitle }}</div>
        <div class="release-list">
          <div v-if="dateDetailReleases.length === 0" class="empty">{{ t('release.no_match') }}</div>
          <ReleaseItem
            v-for="release in dateDetailReleases"
            :key="release.id"
            :release="release"
            @update="emit('update')"
          />
        </div>
      </template>

      <!-- 日历主视图 -->
      <template v-else>
        <ReleaseSearchBar
          v-model="releaseSearch"
          v-model:statusFilter="statusFilter"
          v-model:importanceFilter="importanceFilter"
          v-model:viewMode="viewMode"
          :showSearch="false"
        />
        <div class="calendar-nav">
          <button @click="prevMonth">
            <svg><use href="/icons.svg#chevron-left-icon"/></svg>
          </button>
          <span class="calendar-month-label">{{ monthLabel }}</span>
          <button @click="nextMonth" :disabled="isNextDisabled">
            <svg><use href="/icons.svg#chevron-right-icon"/></svg>
          </button>
        </div>
        <div class="calendar-grid">
          <div v-for="dow in weekDayHeaders" :key="dow" class="calendar-day-header">
            {{ dow }}
          </div>
          <div
            v-for="cell in monthGrid"
            :key="cell.key + (cell.isCurrentMonth ? 'c' : 'p')"
            class="calendar-cell"
            :class="[
              cell.isCurrentMonth ? 'current-month' : 'other-month',
              cell.isToday ? 'today' : '',
              cell.isCurrentMonth ? countClass(cell.count) : '',
            ]"
            @mouseenter="handleCellHover($event, cell)"
            @mouseleave="handleCellLeave"
            @click="handleCellClick(cell)"
          >
            <span class="cell-date">{{ cell.date }}</span>
          </div>
        </div>
      </template>

      <!-- 日历悬浮提示 -->
      <div
        v-if="tooltip"
        class="calendar-tooltip"
        :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
      >
        <div v-for="r in tooltip.releases" :key="r.id" class="tooltip-item">
          <span class="tooltip-repo">{{ r.owner }}/{{ r.repo }}</span>
          <span class="tooltip-tag">{{ r.tag_name }}</span>
        </div>
      </div>
    </template>

    <!-- Repo 级别的右键菜单 -->
    <div v-if="repoContextMenu" class="context-menu" :style="{ left: repoContextMenu.x + 'px', top: repoContextMenu.y + 'px' }" @click.stop>
      <button @click="handleRepoOpenLink">{{ t('context.open') }}</button>
      <button @click="handleRepoCopyLink">{{ t('context.copy_link') }}</button>
    </div>
  </section>
</template>
