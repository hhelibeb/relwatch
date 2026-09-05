<script setup lang="ts">
import { computed } from 'vue'
import type { DonutSegment } from '../composables/useAiUsageStats'
import { formatTokens } from '../composables/useAiUsageStats'
import { t } from '../i18n'

// SVG 圆环图：周长取 100（r = 100 / 2π ≈ 15.915），每段 stroke-dasharray 直接用
// 百分比表达，免去角度/弧长计算；hover 用原生 <title> 提示。
const props = defineProps<{ segments: DonutSegment[]; totalTokens: number }>()

// 固定调色板（亮暗主题均可读）；「其他」固定灰色置于末位
const PALETTE = ['#3b82f6', '#10b981', '#f59e0b', '#ef4444', '#8b5cf6', '#06b6d4', '#84cc16']
const OTHER_COLOR = '#9ca3af'

const R = 15.915

function segColor(index: number, seg: DonutSegment): string {
  if (seg.key === 'other') return OTHER_COLOR
  return PALETTE[index % PALETTE.length]
}

/** 每段起始偏移：25 让 0% 落在 12 点方向，再减去前面各段的累计份额。 */
function dashOffset(index: number): number {
  const prev = props.segments.slice(0, index).reduce((s, x) => s + x.share, 0)
  return 25 - prev * 100
}

const hasData = computed(() => props.segments.some((s) => s.tokens > 0))
</script>

<template>
  <div class="donut-wrap">
    <div class="donut-chart">
      <svg viewBox="0 0 42 42" class="donut-svg">
        <circle class="donut-ring" cx="21" cy="21" :r="R" fill="none" stroke-width="5" />
        <circle
          v-for="(seg, i) in segments"
          :key="seg.key"
          class="donut-seg"
          cx="21"
          cy="21"
          :r="R"
          fill="none"
          stroke-width="5"
          :stroke="segColor(i, seg)"
          :stroke-dasharray="`${seg.share * 100} ${100 - seg.share * 100}`"
          :stroke-dashoffset="dashOffset(i)"
        >
          <title>{{ seg.label || t('aiUsage.no_source') }} · {{ formatTokens(seg.tokens) }} ({{ (seg.share * 100).toFixed(1) }}%)</title>
        </circle>
      </svg>
      <div class="donut-center">
        <template v-if="hasData">
          <span class="donut-total">{{ formatTokens(totalTokens) }}</span>
          <span class="donut-total-label">{{ t('aiUsage.tokens_unit') }}</span>
        </template>
        <span v-else class="donut-empty">{{ t('aiUsage.empty') }}</span>
      </div>
    </div>
    <ul class="donut-legend">
      <li v-for="(seg, i) in segments" :key="seg.key" class="donut-legend-item">
        <i class="donut-swatch" :style="{ background: segColor(i, seg) }"></i>
        <span class="donut-legend-label" :title="seg.label">{{ seg.label || t('aiUsage.no_source') }}</span>
        <span class="donut-legend-share">{{ (seg.share * 100).toFixed(1) }}%</span>
        <span class="donut-legend-tokens">{{ formatTokens(seg.tokens) }}</span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.donut-wrap {
  display: flex;
  align-items: center;
  gap: 24px;
  padding: 12px 4px;
}

.donut-chart {
  position: relative;
  width: 170px;
  height: 170px;
  flex-shrink: 0;
}

.donut-svg {
  width: 100%;
  height: 100%;
  transform: rotate(0deg);
}

.donut-ring {
  stroke: var(--bg-subtle);
}

.donut-seg:hover {
  stroke-width: 6;
}

.donut-center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  pointer-events: none;
}

.donut-total {
  font-size: 20px;
  font-weight: 700;
  color: var(--text);
}

.donut-total-label {
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 2px;
}

.donut-empty {
  font-size: 12px;
  color: var(--text-muted);
  max-width: 90px;
  text-align: center;
}

.donut-legend {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.donut-legend-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}

.donut-swatch {
  width: 10px;
  height: 10px;
  border-radius: 3px;
  flex-shrink: 0;
}

.donut-legend-label {
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}

.donut-legend-share {
  color: var(--text-muted);
  min-width: 52px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.donut-legend-tokens {
  color: var(--text-muted);
  min-width: 56px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
</style>
