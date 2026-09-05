import { computed, ref, watch } from 'vue'
import { getAiUsageStats } from '../api/aiUsage'
import type { AiUsageActionRow, AiUsageSourceRow, AiUsageStats } from '../api/aiUsage'
import type { Source } from '../api/sources'
import { listSources } from '../api/sources'
import { toDateKey } from '../utils/dateKey'

// ── 纯函数（供组件与单测复用） ────────────────────────────────────────

/** 单日 token 合计（输入 + 输出；缓存命中是输入的子集，不重复计）。 */
export function dailyTotalTokens(d: { prompt_tokens: number; completion_tokens: number }): number {
  return d.prompt_tokens + d.completion_tokens
}

/** token 数缩写展示：1.2M / 34.5K / 890（饼图中心与汇总用）。 */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return String(n)
}

export interface HeatCell {
  day: string
  tokens: number
  calls: number
  /** 0=无消耗，1-4 由当日 tokens 相对窗口最大值的四分位分档 */
  level: number
  /** 晚于 today 的占位格（渲染为透明） */
  isFuture: boolean
}

export interface HeatmapData {
  /** 周列（周日 → 周六），GitHub 贡献图布局 */
  weeks: HeatCell[][]
  maxTokens: number
}

/** 构建以 today 所在周为末列、向前 maxWeeks 周的热力图格子。
 *  不足 maxWeeks（窗口短）时按实际跨度生成；daily 里没有的日期补空格。 */
export function buildHeatmap(
  daily: { day: string; prompt_tokens: number; completion_tokens: number; calls: number }[],
  today: Date,
  maxWeeks = 53,
): HeatmapData {
  const byDay = new Map(daily.map((d) => [d.day, d]))
  const todayKey = toDateKey(today)

  // 末列 = today 所在周（列首为周日）；起点对齐 maxWeeks 周前的周日
  const todayDow = today.getDay()
  const lastWeekStart = new Date(today)
  lastWeekStart.setDate(today.getDate() - todayDow)
  const firstWeekStart = new Date(lastWeekStart)
  firstWeekStart.setDate(lastWeekStart.getDate() - (maxWeeks - 1) * 7)

  const weeks: HeatCell[][] = []
  let maxTokens = 0
  for (let w = 0; w < maxWeeks; w++) {
    const col: HeatCell[] = []
    for (let d = 0; d < 7; d++) {
      const cell = new Date(firstWeekStart)
      cell.setDate(firstWeekStart.getDate() + w * 7 + d)
      const key = toDateKey(cell)
      const isFuture = key > todayKey
      const entry = byDay.get(key)
      const tokens = entry ? dailyTotalTokens(entry) : 0
      if (tokens > maxTokens) maxTokens = tokens
      col.push({ day: key, tokens, calls: entry?.calls ?? 0, level: 0, isFuture })
    }
    weeks.push(col)
  }
  // 二次遍历分档（maxTokens 需先算出）
  for (const col of weeks) {
    for (const cell of col) {
      cell.level = heatLevel(cell.tokens, maxTokens)
    }
  }
  return { weeks, maxTokens }
}

/** 色阶分档：0=无消耗；1-4 按相对窗口最大值的四分位。 */
export function heatLevel(tokens: number, maxTokens: number): number {
  if (tokens <= 0 || maxTokens <= 0) return 0
  const ratio = tokens / maxTokens
  if (ratio <= 0.25) return 1
  if (ratio <= 0.5) return 2
  if (ratio <= 0.75) return 3
  return 4
}

export interface DonutSegment {
  key: string
  label: string
  tokens: number
  /** 0-1，按 tokens 合计占比；总量为 0 时为 0 */
  share: number
}

/** 饼图（按监控源）：按 tokens 降序，前 7 名单独成段、其余并入「其他」。
 *  source_id 为 null 的行（连接测试等无源调用）label 传空，由组件以 i18n 文案兜底。 */
export function aggregateDonutBySource(rows: AiUsageSourceRow[], otherLabel: string, maxSegments = 7): DonutSegment[] {
  const sorted = [...rows]
    .map((r) => ({
      key: r.source_id === null ? 'no-source' : `s-${r.source_id}`,
      label: r.label ?? '',
      tokens: dailyTotalTokens(r),
    }))
    .sort((a, b) => b.tokens - a.tokens)
  return toSegments(sorted, otherLabel, maxSegments)
}

/** 饼图（按操作类型）：摘要 / 翻译 / 语言检测 / 连接测试。label 传 action 原值，由组件映射 i18n。 */
export function aggregateDonutByAction(rows: AiUsageActionRow[], otherLabel: string, maxSegments = 7): DonutSegment[] {
  const sorted = [...rows]
    .map((r) => ({
      key: r.action,
      label: r.action,
      tokens: dailyTotalTokens(r),
    }))
    .sort((a, b) => b.tokens - a.tokens)
  return toSegments(sorted, otherLabel, maxSegments)
}

function toSegments(
  sorted: { key: string; label: string; tokens: number }[],
  otherLabel: string,
  maxSegments: number,
): DonutSegment[] {
  const total = sorted.reduce((s, x) => s + x.tokens, 0)
  const segments: DonutSegment[] = sorted.slice(0, maxSegments).map((x) => ({
    ...x,
    share: total > 0 ? x.tokens / total : 0,
  }))
  const rest = sorted.slice(maxSegments)
  if (rest.length > 0) {
    const restTokens = rest.reduce((s, x) => s + x.tokens, 0)
    segments.push({
      key: 'other',
      label: otherLabel,
      tokens: restTokens,
      share: total > 0 ? restTokens / total : 0,
    })
  }
  return segments
}

/** 表格数据行（按源分组）→ TSV 文本（含表头，制表符分隔可直接贴入 Excel）。 */
export function buildSourceTsv(headers: string[], rows: AiUsageSourceRow[], noSourceLabel: string): string {
  const lines = [headers.join('\t')]
  for (const r of rows) {
    lines.push(
      [
        r.label ?? noSourceLabel,
        String(r.calls),
        String(r.prompt_tokens),
        String(r.completion_tokens),
        String(r.cache_hit_tokens),
        String(r.cache_miss_tokens),
        String(dailyTotalTokens(r)),
      ].join('\t'),
    )
  }
  return lines.join('\n')
}

// ── Composable ──────────────────────────────────────────────────────

/** 时间范围选项（null = 全部）。热力图列数随窗口收窄，「全部」封顶 53 周。 */
export const AI_USAGE_RANGE_OPTIONS: { value: number | null; days: number }[] = [
  { value: 30, days: 30 },
  { value: 90, days: 90 },
  { value: 365, days: 365 },
  { value: null, days: 365 },
]

export function useAiUsageStats() {
  const sourceId = ref<number | null>(null)
  const days = ref<number | null>(365)
  const stats = ref<AiUsageStats | null>(null)
  const loading = ref(false)
  const error = ref('')
  const sources = ref<Source[]>([])

  async function load() {
    loading.value = true
    error.value = ''
    try {
      stats.value = await getAiUsageStats(sourceId.value, days.value)
    } catch (e: unknown) {
      error.value = e instanceof Error ? e.message : String(e)
      stats.value = null
    } finally {
      loading.value = false
    }
  }

  async function loadSources() {
    try {
      sources.value = await listSources()
    } catch {
      // 筛选下拉失败不阻塞主数据；下拉退化为「全部」
      sources.value = []
    }
  }

  watch([sourceId, days], load)
  load()
  loadSources()

  /** 热力图窗口周数：随时间范围收窄，「全部」封顶 53 周。 */
  const heatmap = computed<HeatmapData | null>(() => {
    if (!stats.value) return null
    const option = AI_USAGE_RANGE_OPTIONS.find((o) => o.value === days.value)
    const maxWeeks = Math.min(53, Math.ceil((option?.days ?? 365) / 7) + 1)
    return buildHeatmap(stats.value.daily, new Date(), maxWeeks)
  })

  const totalTokens = computed(() =>
    (stats.value?.daily ?? []).reduce((s, d) => s + dailyTotalTokens(d), 0),
  )
  const totalCalls = computed(() => (stats.value?.daily ?? []).reduce((s, d) => s + d.calls, 0))
  const cacheHitTokens = computed(() =>
    (stats.value?.daily ?? []).reduce((s, d) => s + d.cache_hit_tokens, 0),
  )

  return {
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
    reload: load,
  }
}
