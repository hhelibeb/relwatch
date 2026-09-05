<script setup lang="ts">
import { computed, inject, onMounted, onUnmounted, ref } from 'vue'
import { ShowToastKey } from '../injection-keys'
import AiUsageHeatmap from './AiUsageHeatmap.vue'
import AiUsageDonut from './AiUsageDonut.vue'
import {
  useAiUsageStats,
  buildSourceTsv,
  aggregateDonutBySource,
  aggregateDonutByAction,
  formatTokens,
  dailyTotalTokens,
} from '../composables/useAiUsageStats'
import type { AiUsageSourceRow } from '../api/aiUsage'
import { copyTextToClipboard } from '../api/client'
import { useDragResize } from '../composables/useDragResize'
import { registerOverlayActive } from '../composables/contextMenuBus'
import { track } from '../composables/useUsageTracking'
import { t } from '../i18n'

// AI 词元用量统计弹窗：GitHub 风格日历热力图（按天）+ 饼图/表格切换（按源分组）。
// 数据入口 get_ai_usage_stats：一次返回逐日/按源/按操作三组聚合，筛选条件三者共享。
const emit = defineEmits<{ close: [] }>()

const showToast = inject(ShowToastKey, () => {})

const modalEl = ref<HTMLElement | null>(null)
const { startDrag, startResize } = useDragResize(modalEl, {
  minWidth: 520,
  minHeight: 380,
  persistKey: 'relwatch.ai-usage.rect',
})

const {
  sourceId,
  days,
  stats,
  loading,
  error,
  sources,
  heatmap,
  totalTokens,
  totalCalls,
  cacheHitTokens,
  estimatedTokens,
  reload,
} = useAiUsageStats()

// ── 视图状态 ──
type BottomTab = 'donut' | 'table'
const bottomTab = ref<BottomTab>('table')
type DonutDim = 'source' | 'action'
const donutDim = ref<DonutDim>('source')

const sourceOptions = computed(() =>
  sources.value.map((s) => ({
    id: s.id,
    // 与表格/饼图的统计维度一致：显示 owner/repo 本名（description 是用户备注）
    label: `${s.owner}/${s.repo}`,
  })),
)

const rangeOptions = computed(() => [
  { value: 30 as number | null, label: t('aiUsage.range_30d') },
  { value: 90 as number | null, label: t('aiUsage.range_90d') },
  { value: 365 as number | null, label: t('aiUsage.range_1y') },
  { value: null as number | null, label: t('aiUsage.range_all') },
])

const hasAnyData = computed(() => (stats.value?.daily.length ?? 0) > 0)

// ── 饼图 ──
const donutSegments = computed(() => {
  if (!stats.value) return []
  return donutDim.value === 'source'
    ? aggregateDonutBySource(stats.value.by_source, t('aiUsage.other'))
    : aggregateDonutByAction(stats.value.by_action, t('aiUsage.other'))
})

function donutSegmentLabel(seg: { key: string; label: string }): string {
  if (seg.label) return seg.label
  return t('aiUsage.no_source')
}

// ── 表格 ──
const tableRows = computed(() => stats.value?.by_source ?? [])
const grandTotal = computed(() => tableRows.value.reduce((s, r) => s + dailyTotalTokens(r), 0))

function sourceLabel(r: AiUsageSourceRow): string {
  return r.label ?? t('aiUsage.no_source')
}

function actionLabel(action: string): string {
  return t(`aiUsage.action_${action}`)
}

function rowShare(r: AiUsageSourceRow): string {
  if (grandTotal.value <= 0) return '0%'
  return `${((dailyTotalTokens(r) / grandTotal.value) * 100).toFixed(1)}%`
}

async function copyTable() {
  try {
    const headers = [
      t('aiUsage.col_source'),
      t('aiUsage.col_calls'),
      t('aiUsage.col_prompt'),
      t('aiUsage.col_completion'),
      t('aiUsage.col_cache_hit'),
      t('aiUsage.col_cache_miss'),
      t('aiUsage.col_total'),
    ]
    await copyTextToClipboard(buildSourceTsv(headers, tableRows.value, t('aiUsage.no_source')))
    showToast(t('aiUsage.copied'))
  } catch (e: unknown) {
    showToast(t('release.copy_failed') + (e instanceof Error ? e.message : String(e)))
  }
}

function switchTab(tab: BottomTab) {
  bottomTab.value = tab
}

function handleClose() {
  emit('close')
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    e.stopPropagation()
    handleClose()
  }
}

let unregisterOverlay: (() => void) | null = null

onMounted(() => {
  track('settings.ai_usage_open')
  window.addEventListener('keydown', handleKeydown)
  // 弹窗挂载即视为覆盖层打开：Esc 不应最小化到托盘（供 useEscapeToTray 判定）
  unregisterOverlay = registerOverlayActive(() => true)
})

onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown)
  unregisterOverlay?.()
})
</script>

<template>
  <Teleport to="body">
    <div class="ai-usage-overlay" @click.self="handleClose">
      <div ref="modalEl" class="ai-usage-modal" role="dialog" aria-modal="true">
        <div class="ai-usage-header" @pointerdown="startDrag">
          <span class="ai-usage-title">{{ t('aiUsage.title') }}</span>
          <button class="ai-usage-close" :title="t('release.detail_close')" @click="handleClose">
            <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>
          </button>
        </div>

        <div class="ai-usage-toolbar">
          <label class="ai-usage-filter">
            <span class="ai-usage-filter-label">{{ t('aiUsage.filter_source') }}</span>
            <select v-model.number="sourceId" class="ai-usage-select">
              <option :value="null">{{ t('aiUsage.filter_source_all') }}</option>
              <option v-for="s in sourceOptions" :key="s.id" :value="s.id">{{ s.label }}</option>
            </select>
          </label>
          <label class="ai-usage-filter">
            <span class="ai-usage-filter-label">{{ t('aiUsage.filter_range') }}</span>
            <select v-model.number="days" class="ai-usage-select">
              <option v-for="o in rangeOptions" :key="String(o.value)" :value="o.value">{{ o.label }}</option>
            </select>
          </label>
          <div class="ai-usage-summary">
            <span class="ai-usage-chip" :title="t('aiUsage.col_total')">
              {{ t('aiUsage.tokens_unit') }} <b>{{ formatTokens(totalTokens) }}</b>
            </span>
            <span class="ai-usage-chip" :title="t('aiUsage.col_calls')">
              {{ t('aiUsage.calls_unit') }} <b>{{ totalCalls }}</b>
            </span>
            <span class="ai-usage-chip" :title="t('aiUsage.cache_hit_note')">
              {{ t('aiUsage.cache_hit_short') }} <b>{{ formatTokens(cacheHitTokens) }}</b>
            </span>
            <!-- 估算行（中转剥离 usage 按字符数兜底）混在真实统计里无法分辨，显式标出 -->
            <span
              v-if="estimatedTokens > 0"
              class="ai-usage-chip ai-usage-chip-estimated"
              :title="t('aiUsage.estimated_note', estimatedTokens.toLocaleString())"
            >
              {{ t('aiUsage.estimated_chip', formatTokens(estimatedTokens)) }}
            </span>
          </div>
          <button class="btn-secondary ai-usage-reload" :disabled="loading" @click="reload">{{ t('aiUsage.reload') }}</button>
        </div>

        <div class="ai-usage-body">
          <div v-if="loading" class="ai-usage-state">{{ t('aiUsage.loading') }}</div>
          <div v-else-if="error" class="ai-usage-state ai-usage-error">{{ error }}</div>
          <div v-else-if="!hasAnyData || !heatmap" class="ai-usage-state">{{ t('aiUsage.empty') }}</div>
          <template v-else>
            <AiUsageHeatmap :data="heatmap" />

            <div class="ai-usage-bottom-tabs">
              <div class="ai-usage-tab-buttons">
                <button :class="{ active: bottomTab === 'table' }" @click="switchTab('table')">{{ t('aiUsage.tab_table') }}</button>
                <button :class="{ active: bottomTab === 'donut' }" @click="switchTab('donut')">{{ t('aiUsage.tab_donut') }}</button>
              </div>

              <div v-if="bottomTab === 'table'" class="ai-usage-table-wrap">
                <div class="ai-usage-table-actions">
                  <button class="btn-sm" :disabled="tableRows.length === 0" @click="copyTable">{{ t('aiUsage.copy') }}</button>
                </div>
                <table class="ai-usage-table">
                  <thead>
                    <tr>
                      <th class="left">{{ t('aiUsage.col_source') }}</th>
                      <th>{{ t('aiUsage.col_calls') }}</th>
                      <th>{{ t('aiUsage.col_prompt') }}</th>
                      <th>{{ t('aiUsage.col_completion') }}</th>
                      <th>{{ t('aiUsage.col_cache_hit') }}</th>
                      <th>{{ t('aiUsage.col_cache_miss') }}</th>
                      <th>{{ t('aiUsage.col_total') }}</th>
                      <th>{{ t('aiUsage.col_share') }}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr v-for="r in tableRows" :key="r.source_id ?? 'no-source'">
                      <td class="left" :title="sourceLabel(r)">{{ sourceLabel(r) }}</td>
                      <td>{{ r.calls }}</td>
                      <td>{{ r.prompt_tokens }}</td>
                      <td>{{ r.completion_tokens }}</td>
                      <td>{{ r.cache_hit_tokens }}</td>
                      <td>{{ r.cache_miss_tokens }}</td>
                      <td class="strong">{{ dailyTotalTokens(r) }}</td>
                      <td>{{ rowShare(r) }}</td>
                    </tr>
                  </tbody>
                  <tfoot v-if="tableRows.length > 1">
                    <tr>
                      <td class="left">{{ t('aiUsage.total_label') }}</td>
                      <td>{{ totalCalls }}</td>
                      <td colspan="4"></td>
                      <td class="strong">{{ grandTotal }}</td>
                      <td>100%</td>
                    </tr>
                  </tfoot>
                </table>
              </div>

              <div v-else class="ai-usage-donut-wrap">
                <div class="ai-usage-dim-buttons">
                  <button :class="{ active: donutDim === 'source' }" @click="donutDim = 'source'">{{ t('aiUsage.dim_source') }}</button>
                  <button :class="{ active: donutDim === 'action' }" @click="donutDim = 'action'">{{ t('aiUsage.dim_action') }}</button>
                </div>
                <AiUsageDonut
                  :segments="donutSegments.map((s) => ({ ...s, label: donutSegmentLabel(s) }))"
                  :total-tokens="totalTokens"
                />
                <p v-if="donutDim === 'action'" class="ai-usage-dim-hint">{{ t('aiUsage.dim_action_hint', actionLabel('translate'), actionLabel('summary')) }}</p>
              </div>
            </div>
          </template>
        </div>

        <div
          v-for="dir in ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw']"
          :key="dir"
          class="resize-handle"
          :class="`resize-handle-${dir}`"
          @pointerdown="startResize($event, dir as 'n')"
        ></div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.ai-usage-overlay {
  position: fixed;
  inset: 0;
  z-index: 10010;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  padding: 24px;
}

.ai-usage-modal {
  position: relative;
  display: flex;
  flex-direction: column;
  width: min(760px, 100%);
  max-height: calc(100vh - 48px);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.ai-usage-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  cursor: move;
  user-select: none;
}

.ai-usage-title {
  font-size: 15px;
  font-weight: 600;
}

.ai-usage-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 6px;
  cursor: pointer;
}

.ai-usage-close:hover {
  background: var(--bg-subtle);
  color: var(--text);
}

.ai-usage-close svg {
  width: 14px;
  height: 14px;
}

.ai-usage-toolbar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  padding: 10px 16px;
  border-bottom: 1px solid var(--border);
}

.ai-usage-filter {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-muted);
}

.ai-usage-select {
  max-width: 200px;
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
}

.ai-usage-summary {
  display: inline-flex;
  gap: 6px;
  margin-left: auto;
}

.ai-usage-chip {
  font-size: 11px;
  color: var(--text-muted);
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 2px 10px;
  white-space: nowrap;
}

.ai-usage-chip b {
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

/* 估算 chip：虚线边框与真实统计的 chip 区分 */
.ai-usage-chip-estimated {
  border-style: dashed;
}

.ai-usage-reload {
  font-size: 12px;
  padding: 4px 10px;
}

.ai-usage-body {
  flex: 1;
  overflow-y: auto;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ai-usage-state {
  padding: 40px 0;
  text-align: center;
  color: var(--text-muted);
  font-size: 13px;
}

.ai-usage-error {
  color: var(--danger, #dc2626);
}

.ai-usage-bottom-tabs {
  border-top: 1px solid var(--border);
  padding-top: 10px;
}

.ai-usage-tab-buttons,
.ai-usage-dim-buttons {
  display: inline-flex;
  gap: 0;
  border: 1px solid var(--border);
  border-radius: 8px;
  overflow: hidden;
}

.ai-usage-tab-buttons button,
.ai-usage-dim-buttons button {
  padding: 4px 14px;
  font-size: 12px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}

.ai-usage-tab-buttons button.active,
.ai-usage-dim-buttons button.active {
  background: var(--primary);
  color: #fff;
}

.ai-usage-table-wrap {
  margin-top: 10px;
}

.ai-usage-table-actions {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 6px;
}

.ai-usage-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.ai-usage-table th,
.ai-usage-table td {
  padding: 5px 8px;
  border-bottom: 1px solid var(--border);
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.ai-usage-table th.left,
.ai-usage-table td.left {
  text-align: left;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ai-usage-table th {
  color: var(--text-muted);
  font-weight: 600;
  font-size: 11px;
}

.ai-usage-table td.strong {
  font-weight: 700;
}

.ai-usage-table tfoot td {
  font-weight: 600;
  border-top: 1px solid var(--border);
}

.ai-usage-donut-wrap {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.ai-usage-dim-hint {
  margin: 0;
  font-size: 11px;
  color: var(--text-muted);
}

/* resize 手柄：与 ReleaseDetailModal 同款八向布局 */
.resize-handle {
  position: absolute;
  z-index: 10;
}

.resize-handle-n { top: -3px; left: 8px; right: 8px; height: 6px; cursor: n-resize; }
.resize-handle-s { bottom: -3px; left: 8px; right: 8px; height: 6px; cursor: s-resize; }
.resize-handle-e { right: -3px; top: 8px; bottom: 8px; width: 6px; cursor: e-resize; }
.resize-handle-w { left: -3px; top: 8px; bottom: 8px; width: 6px; cursor: w-resize; }
.resize-handle-ne { top: -3px; right: -3px; width: 10px; height: 10px; cursor: ne-resize; }
.resize-handle-nw { top: -3px; left: -3px; width: 10px; height: 10px; cursor: nw-resize; }
.resize-handle-se { bottom: -3px; right: -3px; width: 10px; height: 10px; cursor: se-resize; }
.resize-handle-sw { bottom: -3px; left: -3px; width: 10px; height: 10px; cursor: sw-resize; }
</style>
