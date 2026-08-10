<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { getLocale, t } from '../i18n'
import { toDateKey } from '../utils/dateKey'
import { track } from '../composables/useUsageTracking'

const props = defineProps<{
  releases: ReleaseInfo[]
  year: number
  month: number
}>()

const emit = defineEmits<{
  prevMonth: []
  nextMonth: []
  selectDate: [date: string]
}>()

interface CalendarCell {
  date: number
  key: string
  count: number
  isCurrentMonth: boolean
  isToday: boolean
  releases: ReleaseInfo[]
}

const tooltip = ref<{ x: number; y: number; releases: ReleaseInfo[] } | null>(null)

const calendarMap = computed(() => {
  const map = new Map<string, ReleaseInfo[]>()
  for (const release of props.releases) {
    const key = toDateKey(new Date(release.published_at))
    if (!map.has(key)) map.set(key, [])
    map.get(key)!.push(release)
  }
  return map
})

const todayKey = toDateKey(new Date())

// 周起始按语言习惯：中文周一，英文周日
const weekStartsOnMonday = computed(() => getLocale() === 'zh-CN')

const weekDayHeaders = computed(() => {
  if (weekStartsOnMonday.value) return ['一', '二', '三', '四', '五', '六', '日']
  return ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
})

const monthLabel = computed(() => {
  const d = new Date(props.year, props.month - 1, 1)
  const locale = getLocale()
  return d.toLocaleDateString(locale, { year: 'numeric', month: 'long' })
})

const monthGrid = computed(() => {
  const year = props.year
  const month = props.month
  const firstDay = new Date(year, month - 1, 1)
  const lastDay = new Date(year, month, 0)
  const totalDays = lastDay.getDate()
  // getDay() 以周日为 0；周一起始时转换为周一为 0
  const startDow = weekStartsOnMonday.value ? (firstDay.getDay() + 6) % 7 : firstDay.getDay()

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
  tooltip.value = { x: e.clientX + 12, y: e.clientY + 12, releases: cell.releases }
}

function handleCellLeave() {
  tooltip.value = null
}

function handleCellClick(cell: CalendarCell) {
  if (!cell.isCurrentMonth || cell.count === 0) return
  tooltip.value = null
  track('calendar.select_date')
  emit('selectDate', cell.key)
}

function handlePrevMonth() {
  track('calendar.prev_month')
  emit('prevMonth')
}

function handleNextMonth() {
  track('calendar.next_month')
  emit('nextMonth')
}
</script>

<template>
  <div class="calendar-nav">
    <button @click="handlePrevMonth">
      <svg><use href="/icons.svg#chevron-left-icon"/></svg>
    </button>
    <span class="calendar-month-label">{{ monthLabel }}</span>
    <button @click="handleNextMonth" :disabled="props.year >= new Date().getFullYear() && props.month >= new Date().getMonth() + 1">
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
      <span v-if="cell.isCurrentMonth && cell.count > 0" class="cell-count">{{ cell.count }}</span>
    </div>
  </div>
  <!-- 热力图例：色阶与当日版本数量的对应关系 -->
  <div class="calendar-legend">
    <span class="calendar-legend-label">{{ t('release.calendar_legend') }}</span>
    <span class="calendar-legend-item"><i class="legend-swatch legend-swatch-1"></i>1</span>
    <span class="calendar-legend-item"><i class="legend-swatch legend-swatch-2"></i>2</span>
    <span class="calendar-legend-item"><i class="legend-swatch legend-swatch-3"></i>3</span>
    <span class="calendar-legend-item"><i class="legend-swatch legend-swatch-4"></i>4+</span>
  </div>

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

<style scoped>
.calendar-nav {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  margin-bottom: 12px;
}

.calendar-nav button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
  transition: background 0.1s;
}

.calendar-nav button:hover:not(:disabled) {
  background: var(--bg-subtle);
}

.calendar-nav button:disabled {
  opacity: 0.35;
  cursor: default;
}

.calendar-nav button svg {
  width: 14px;
  height: 14px;
}

.calendar-nav .calendar-month-label {
  font-size: 15px;
  font-weight: 600;
  min-width: 120px;
  text-align: center;
}

.calendar-grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 2px;
  background: transparent;
  border-radius: var(--radius);
  border: 1px solid var(--border);
  padding: 8px;
}

.calendar-day-header {
  text-align: center;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
  padding: 4px 0;
}

.calendar-cell {
  aspect-ratio: 17 / 14;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  cursor: default;
  font-size: 13px;
  position: relative;
  transition: background 0.1s;
  color: var(--text);
}

.calendar-cell.current-month {
  cursor: pointer;
}

.calendar-cell.current-month:hover {
  background: var(--bg-subtle);
}

.calendar-cell.other-month {
  color: var(--text-muted);
  opacity: 0.4;
}

.calendar-cell.today {
  font-weight: 700;
  box-shadow: inset 0 0 0 2px var(--primary);
}

.calendar-cell .cell-date {
  font-size: 13px;
  line-height: 1;
}

/* 格内版本数量：配合热力底色让数值一目了然 */
.calendar-cell .cell-count {
  margin-top: 3px;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
  color: var(--text);
  opacity: 0.75;
}

/* 热力图例 */
.calendar-legend {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 8px;
  font-size: 11px;
  color: var(--text-muted);
}

.calendar-legend-label {
  margin-right: 2px;
}

.calendar-legend-item {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.legend-swatch {
  display: inline-block;
  width: 12px;
  height: 12px;
  border-radius: 3px;
  border: 1px solid var(--border);
}

.legend-swatch-1 { background: var(--heat-1); }
.legend-swatch-2 { background: var(--heat-2); }
.legend-swatch-3 { background: var(--heat-3); }
.legend-swatch-4 { background: var(--heat-4); }

.calendar-cell-count-1 {
  background: var(--heat-1);
}

.calendar-cell-count-2 {
  background: var(--heat-2);
}

.calendar-cell-count-3 {
  background: var(--heat-3);
}

.calendar-cell-count-4 {
  background: var(--heat-4);
}

.calendar-cell-count-1:hover,
.calendar-cell-count-2:hover,
.calendar-cell-count-3:hover,
.calendar-cell-count-4:hover {
  filter: brightness(0.95);
}

:global([data-theme="dark"] .calendar-cell-count-1:hover),
:global([data-theme="dark"] .calendar-cell-count-2:hover),
:global([data-theme="dark"] .calendar-cell-count-3:hover),
:global([data-theme="dark"] .calendar-cell-count-4:hover) {
  filter: brightness(1.2);
}

.calendar-tooltip {
  position: fixed;
  z-index: 10001;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  box-shadow: var(--shadow-lg);
  font-size: 12px;
  max-width: 260px;
  pointer-events: none;
}

.calendar-tooltip .tooltip-item {
  padding: 2px 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.calendar-tooltip .tooltip-repo {
  color: var(--text-muted);
  margin-right: 6px;
}

.calendar-tooltip .tooltip-tag {
  font-weight: 600;
  color: var(--primary);
}


</style>
