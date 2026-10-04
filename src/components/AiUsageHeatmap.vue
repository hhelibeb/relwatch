<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { HeatCell, HeatmapData } from '../composables/useAiUsageStats'
import { heatLevel } from '../composables/useAiUsageStats'
import { parseDateKey } from '../utils/dateKey'
import { getLocale, t } from '../i18n'

// GitHub 贡献图风格热力图：列=周（周日→周六），行=周几。
// 色阶复用 ReleaseCalendar 的 --heat-1..4 变量；悬停显示当日用量。
const props = defineProps<{ data: HeatmapData }>()

const CELL = 11
const GAP = 3

const TIP_GAP = 12
const VIEWPORT_PAD = 4

const scrollEl = ref<HTMLElement | null>(null)
const tooltipEl = ref<HTMLElement | null>(null)
const tooltip = ref<{ x: number; y: number; day: string; tokens: number; calls: number } | null>(null)

// 一年 53 列远超弹窗宽度：初始与数据变化后都滚到最右端（今天所在周），
// 否则默认停在一年前的空窗口，最近的消耗反而看不到。
function scrollToToday() {
  if (scrollEl.value) scrollEl.value.scrollLeft = scrollEl.value.scrollWidth
}

onMounted(() => nextTick(scrollToToday))
watch(() => props.data, () => nextTick(scrollToToday))

const columns = computed(() => props.data.weeks.length)

/** 每列的月份标签：仅当该列包含某月 1 日时打标（与 GitHub 一致）。 */
const monthLabels = computed(() => {
  const labels: { col: number; label: string }[] = []
  props.data.weeks.forEach((week, col) => {
    for (const cell of week) {
      const d = parseDateKey(cell.day)
      if (d.getDate() === 1) {
        labels.push({ col, label: d.toLocaleDateString(getLocale(), { month: 'short' }) })
        break
      }
    }
  })
  return labels
})

// 周几行标：GitHub 只在第 1/3/5 行打标，避免过密
const dowLabels = computed(() => {
  const zh = getLocale() === 'zh-CN'
  const names = zh ? ['日', '一', '二', '三', '四', '五', '六'] : ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
  return [
    { row: 0, label: names[0] },
    { row: 2, label: names[2] },
    { row: 4, label: names[4] },
  ]
})

function formatDay(day: string): string {
  return parseDateKey(day).toLocaleDateString(getLocale(), { month: 'short', day: 'numeric' })
}

function handleHover(e: MouseEvent, cell: HeatCell) {
  if (cell.isFuture) return // future 占位格不显示 tooltip（不依赖 CSS visibility 挡事件）
  tooltip.value = {
    x: e.clientX + TIP_GAP,
    y: e.clientY + TIP_GAP,
    day: cell.day,
    tokens: cell.tokens,
    calls: cell.calls,
  }
  // 光标贴近右/下边界时提示框会超出窗口（贴右边界的热力图列最常碰到）：
  // 渲染完量到实际尺寸后翻到光标另一侧，再兜底钳进窗口（同 SourceTab 健康提示的做法）
  nextTick(() => {
    const el = tooltipEl.value
    if (!el || !tooltip.value) return
    const rect = el.getBoundingClientRect()
    if (rect.right > window.innerWidth - VIEWPORT_PAD) tooltip.value.x = e.clientX - rect.width - TIP_GAP
    if (rect.bottom > window.innerHeight - VIEWPORT_PAD) tooltip.value.y = e.clientY - rect.height - TIP_GAP
    tooltip.value.x = Math.max(VIEWPORT_PAD, tooltip.value.x)
    tooltip.value.y = Math.max(VIEWPORT_PAD, tooltip.value.y)
  })
}

function clearTooltip() {
  tooltip.value = null
}

function levelClass(cell: { tokens: number; isFuture: boolean }): string {
  if (cell.isFuture) return 'heat-future'
  return `heat-${heatLevel(cell.tokens, props.data.maxTokens)}`
}
</script>

<template>
  <div ref="scrollEl" class="heatmap-scroll">
    <div class="heatmap-inner">
      <div class="heatmap-months" :style="{ gridTemplateColumns: `repeat(${columns}, ${CELL}px)` }">
        <span
          v-for="m in monthLabels"
          :key="m.col"
          class="heatmap-month"
          :style="{ gridColumn: m.col + 1 }"
        >{{ m.label }}</span>
      </div>
      <div class="heatmap-body">
        <div class="heatmap-dows">
          <span
            v-for="d in dowLabels"
            :key="d.row"
            class="heatmap-dow"
            :style="{ top: d.row * (CELL + GAP) + 'px' }"
          >{{ d.label }}</span>
        </div>
        <div
          class="heatmap-grid"
          :style="{ gridTemplateColumns: `repeat(${columns}, ${CELL}px)`, gridAutoFlow: 'column', gridTemplateRows: `repeat(7, ${CELL}px)` }"
        >
          <template v-for="(week, wi) in data.weeks" :key="wi">
            <div
              v-for="cell in week"
              :key="cell.day"
              class="heatmap-cell"
              :class="levelClass(cell)"
              @mouseenter="handleHover($event, cell)"
              @mouseleave="clearTooltip"
            ></div>
          </template>
        </div>
      </div>
    </div>
  </div>
  <div class="heatmap-legend">
    <span class="heatmap-legend-label">{{ t('aiUsage.legend_less') }}</span>
    <i class="heatmap-cell heat-0"></i>
    <i class="heatmap-cell heat-1"></i>
    <i class="heatmap-cell heat-2"></i>
    <i class="heatmap-cell heat-3"></i>
    <i class="heatmap-cell heat-4"></i>
    <span class="heatmap-legend-label">{{ t('aiUsage.legend_more') }}</span>
  </div>
  <!-- 挂到 body：宿主弹窗用 transform 做拖动/尺寸补偿，transform 会成为 fixed 后代的
       包含块，left/top 被当成相对弹窗的坐标，且被弹窗的 overflow:hidden 裁掉。 -->
  <Teleport to="body">
    <div
      v-if="tooltip"
      ref="tooltipEl"
      class="heatmap-tooltip"
      :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
    >
      <div class="heatmap-tooltip-day">{{ formatDay(tooltip.day) }}</div>
      <div class="heatmap-tooltip-detail">
        {{ t('aiUsage.fmt_tokens', String(tooltip.tokens)) }} · {{ t('aiUsage.fmt_calls', String(tooltip.calls)) }}
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.heatmap-scroll {
  /* 宿主弹窗 body 是 flex 列：不锁定自身高度时，内容超高会把热力图压扁并
     在热力图内部生出垂直滚动条（overflow-x 非 visible 会连带把 overflow-y 算成 auto）。
     滚动交给弹窗 body 自己。 */
  flex: none;
  overflow-x: auto;
  padding: 4px 2px;
}

.heatmap-inner {
  display: inline-flex;
  flex-direction: column;
  gap: 4px;
}

.heatmap-months {
  display: grid;
  gap: 0 3px;
  margin-left: 26px;
  font-size: 10px;
  color: var(--text-muted);
  line-height: 1;
}

.heatmap-month {
  white-space: nowrap;
}

.heatmap-body {
  display: flex;
  gap: 4px;
}

.heatmap-dows {
  position: relative;
  width: 22px;
  height: calc(7 * 11px + 6 * 3px);
  font-size: 10px;
  color: var(--text-muted);
}

.heatmap-dow {
  position: absolute;
  left: 0;
  line-height: 11px;
  white-space: nowrap;
}

.heatmap-grid {
  display: grid;
  gap: 3px;
}

.heatmap-cell {
  width: 11px;
  height: 11px;
  border-radius: 2px;
  background: var(--heat-1);
}

.heatmap-cell.heat-0 {
  background: var(--bg-subtle);
  border: 1px solid var(--border);
}

.heatmap-cell.heat-1 { background: var(--heat-1); }
.heatmap-cell.heat-2 { background: var(--heat-2); }
.heatmap-cell.heat-3 { background: var(--heat-3); }
.heatmap-cell.heat-4 { background: var(--heat-4); }

.heatmap-cell:not(.heat-future):hover {
  outline: 1px solid var(--primary);
  outline-offset: -1px;
}

.heatmap-cell.heat-future {
  background: transparent;
  border: none;
  visibility: hidden;
}

.heatmap-legend {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
  margin-top: 6px;
  font-size: 11px;
  color: var(--text-muted);
}

.heatmap-legend .heatmap-cell {
  display: inline-block;
}

.heatmap-legend-label {
  margin: 0 4px;
}

.heatmap-tooltip {
  position: fixed;
  z-index: 10020;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 6px 10px;
  box-shadow: var(--shadow-lg);
  font-size: 12px;
  pointer-events: none;
  white-space: nowrap;
}

.heatmap-tooltip-day {
  font-weight: 600;
  margin-bottom: 2px;
}

.heatmap-tooltip-detail {
  color: var(--text-muted);
}
</style>
