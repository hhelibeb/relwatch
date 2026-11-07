<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { getLocale, t } from '../i18n'
import { toDateKey, CALENDAR_MIN_YEAR, calendarMonthAhead } from '../utils/dateKey'
import { useDropdown } from '../composables/useDropdown'
import { track } from '../composables/useUsageTracking'

const props = defineProps<{
  releases: ReleaseInfo[]
  year: number
  month: number
}>()

const emit = defineEmits<{
  prevMonth: []
  nextMonth: []
  jumpTo: [year: number, month: number]
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

// ── 年/月快速跳转 ──────────────
// 当前年月必须现取：本组件常驻挂载（App 用 v-show 切 tab），挂载时求值会在跨月后
// 与按需取时的 calendarMonthAhead 分叉（「下一月」可用、「回今天」却跳回上个月）。
function todayMonth(): { year: number; month: number } {
  const now = new Date()
  return { year: now.getFullYear(), month: now.getMonth() + 1 }
}
const isCurrentMonth = computed(() => {
  const today = todayMonth()
  return props.year === today.year && props.month === today.month
})
const prevMonthDisabled = computed(() => props.year <= CALENDAR_MIN_YEAR && props.month === 1)
// 禁用条件看的是「翻页目标月」而非当前显示月：显示到当前月时，下一月已越过上限
const nextMonthDisabled = computed(() =>
  props.month === 12 ? calendarMonthAhead(props.year + 1, 1) : calendarMonthAhead(props.year, props.month + 1),
)
const nextYearDisabled = computed(() => calendarMonthAhead(props.year + 1, props.month))

function handlePrevYear() {
  track('calendar.prev_year')
  emit('jumpTo', props.year - 1, props.month)
}

function handleNextYear() {
  track('calendar.next_year')
  emit('jumpTo', props.year + 1, props.month)
}

function handleToday() {
  track('calendar.today')
  const today = todayMonth()
  emit('jumpTo', today.year, today.month)
}

const pickerOpen = ref(false)
const picker = useDropdown({ openState: pickerOpen, closedKey: false, hoverOpen: false })
const pickerWrapEl = ref<HTMLElement | null>(null)
const pickerYear = ref(props.year)
const pickerYearPrevDisabled = computed(() => pickerYear.value <= CALENDAR_MIN_YEAR)
const pickerYearNextDisabled = computed(() => pickerYear.value >= todayMonth().year)

const pickerMonths = computed(() =>
  Array.from({ length: 12 }, (_, i) => {
    const m = i + 1
    return {
      month: m,
      label: new Date(pickerYear.value, i, 1).toLocaleDateString(getLocale(), { month: 'short' }),
      disabled: calendarMonthAhead(pickerYear.value, m),
      selected: pickerYear.value === props.year && m === props.month,
    }
  }),
)

// 点击面板外关闭（useDropdown 不管 click-outside）；目标在面板内时不处理
function onDocClick(e: MouseEvent) {
  if (pickerWrapEl.value && !pickerWrapEl.value.contains(e.target as Node)) picker.close()
}

// 打开时把面板年份对齐到当前显示月，并挂上 click-outside 监听
watch(pickerOpen, (open) => {
  if (open) {
    pickerYear.value = props.year
    document.addEventListener('click', onDocClick)
  } else {
    document.removeEventListener('click', onDocClick)
  }
})
onUnmounted(() => document.removeEventListener('click', onDocClick))

function pickMonth(month: number) {
  if (calendarMonthAhead(pickerYear.value, month)) return
  track('calendar.jump_month')
  emit('jumpTo', pickerYear.value, month)
  picker.close()
}
</script>

<template>
  <div class="calendar-nav">
    <button class="calendar-year-btn" :disabled="props.year <= CALENDAR_MIN_YEAR" :title="t('release.calendar_prev_year')" @click="handlePrevYear">«</button>
    <button :disabled="prevMonthDisabled" :title="t('release.calendar_prev_month')" @click="handlePrevMonth">
      <svg><use href="/icons.svg#chevron-left-icon"/></svg>
    </button>
    <div ref="pickerWrapEl" class="calendar-picker-wrap">
      <button
        type="button"
        class="calendar-month-label"
        :aria-expanded="pickerOpen"
        aria-haspopup="menu"
        :title="t('release.calendar_pick_month')"
        @click="picker.toggle($event, true)"
        @keydown="picker.handleTriggerKeydown($event, true)"
      >{{ monthLabel }}</button>
      <div v-if="pickerOpen" class="dropdown-panel calendar-picker" role="menu" @keydown="picker.handleDropdownKeydown">
        <div class="calendar-picker-year">
          <button type="button" :disabled="pickerYearPrevDisabled" @click="pickerYear--">«</button>
          <span>{{ pickerYear }}</span>
          <button type="button" :disabled="pickerYearNextDisabled" @click="pickerYear++">»</button>
        </div>
        <div class="calendar-picker-months">
          <button
            v-for="m in pickerMonths"
            :key="m.month"
            type="button"
            role="menuitem"
            :disabled="m.disabled"
            :class="{ selected: m.selected }"
            @click="pickMonth(m.month)"
          >{{ m.label }}</button>
        </div>
      </div>
    </div>
    <button :disabled="nextMonthDisabled" :title="t('release.calendar_next_month')" @click="handleNextMonth">
      <svg><use href="/icons.svg#chevron-right-icon"/></svg>
    </button>
    <button class="calendar-year-btn" :disabled="nextYearDisabled" :title="t('release.calendar_next_year')" @click="handleNextYear">»</button>
    <button type="button" class="calendar-today-btn" :disabled="isCurrentMonth" @click="handleToday">{{ t('release.calendar_today') }}</button>
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
  border-radius: var(--radius);
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out);
}

.calendar-nav button:hover:not(:disabled) {
  background: var(--hover-surface);
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
  font-size: var(--fs-title);
  font-weight: 600;
  min-width: 120px;
  text-align: center;
}

/* 月份标签是可点击的跳转入口，但视觉上保持纯文本标签 */
.calendar-nav button.calendar-month-label {
  width: auto;
  height: auto;
  padding: 2px 8px;
  border-color: transparent;
  background: transparent;
}

.calendar-nav button.calendar-month-label:hover:not(:disabled) {
  background: var(--hover-surface);
}

.calendar-nav .calendar-today-btn {
  width: auto;
  padding: 0 10px;
  font-size: var(--fs-meta);
}

.calendar-picker-wrap {
  position: relative;
  display: inline-flex;
}

.calendar-picker {
  top: calc(100% + 4px);
  left: 50%;
  transform: translateX(-50%);
  padding: 8px;
  width: 220px;
}

.calendar-picker-year {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
  font-weight: 600;
}

.calendar-picker-year button {
  width: 24px;
  height: 24px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
}

.calendar-picker-year button:disabled {
  opacity: 0.35;
  cursor: default;
}

.calendar-picker-months {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
}

.calendar-picker-months button {
  padding: 6px 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: var(--fs-meta);
  cursor: pointer;
}

.calendar-picker-months button:hover:not(:disabled) {
  background: var(--hover-surface);
}

.calendar-picker-months button.selected {
  background: var(--primary);
  color: var(--on-primary);
}

.calendar-picker-months button:disabled {
  opacity: 0.35;
  cursor: default;
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
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--text-muted);
  padding: 4px 0;
}

.calendar-cell {
  height: 58px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius);
  cursor: default;
  font-size: var(--fs-body);
  position: relative;
  transition: background var(--dur-hover) var(--ease-out);
  color: var(--text);
}

.calendar-cell.current-month {
  cursor: pointer;
}

.calendar-cell.current-month:hover {
  background: var(--hover-surface);
}

.calendar-cell.other-month {
  color: var(--text-muted);
  opacity: 0.4;
}

.calendar-cell.today {
  font-weight: 700;
}

.calendar-cell .cell-date {
  font-size: var(--fs-body);
  line-height: 1;
}

/* 今日用反色药丸标记：热力填充已占满整格，描边叠在同色系蓝底上只有 2:1 */
.calendar-cell.today .cell-date {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 22px;
  height: 18px;
  padding: 0 5px;
  border-radius: var(--radius-pill);
  background: var(--primary);
  color: var(--on-primary);
}

/* 格内版本数量：不再压 0.75 透明度，否则在 --heat-4 上只剩 4.3:1（深色 3.2:1） */
.calendar-cell .cell-count {
  margin-top: 4px;
  font-size: var(--fs-caption);
  font-weight: 600;
  line-height: 1;
  color: var(--text);
}

.calendar-legend {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 8px;
  font-size: var(--fs-caption);
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
  border-radius: var(--radius-xs);
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
  z-index: var(--z-popover);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  box-shadow: var(--shadow-lg);
  font-size: var(--fs-meta);
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
