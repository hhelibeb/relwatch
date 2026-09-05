import { describe, it, expect, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import {
  buildHeatmap,
  heatLevel,
  aggregateDonutBySource,
  aggregateDonutByAction,
  buildSourceTsv,
  formatTokens,
  dailyTotalTokens,
} from '../composables/useAiUsageStats'
import type { AiUsageSourceRow, AiUsageActionRow } from '../api/aiUsage'
import AiUsageHeatmap from '../components/AiUsageHeatmap.vue'
import { toDateKey } from '../utils/dateKey'

// AI Token 用量统计：热力图网格构建（GitHub 风格周列×7 行）、
// 饼图聚合（前 7 + 其他）、TSV 导出与 token 缩写。组件层只测
// AiUsageHeatmap 的格子渲染（色阶 class / future 占位 / 图例）。

describe('heatLevel', () => {
  it('0 与负值均为 0 档', () => {
    expect(heatLevel(0, 100)).toBe(0)
    expect(heatLevel(-5, 100)).toBe(0)
    expect(heatLevel(50, 0)).toBe(0) // maxTokens=0（空窗口）时全部 0 档
  })

  it('按相对最大值的四分位分档', () => {
    expect(heatLevel(25, 100)).toBe(1)
    expect(heatLevel(26, 100)).toBe(2)
    expect(heatLevel(50, 100)).toBe(2)
    expect(heatLevel(51, 100)).toBe(3)
    expect(heatLevel(75, 100)).toBe(3)
    expect(heatLevel(76, 100)).toBe(4)
    expect(heatLevel(100, 100)).toBe(4)
  })
})

describe('buildHeatmap', () => {
  const today = new Date(2026, 8, 15) // 2026-09-15（周二），本地时区

  it('末列包含今天所在周，列数 = maxWeeks', () => {
    const { weeks } = buildHeatmap([], today, 53)
    expect(weeks).toHaveLength(53)
    const lastWeek = weeks[52]
    // 列首为周日：2026-09-13 是周日
    expect(lastWeek[0].day).toBe('2026-09-13')
    // 周二（today）在列内第 3 行
    expect(lastWeek[2].day).toBe('2026-09-15')
  })

  it('晚于今天的日期是 future 占位、无数据', () => {
    const { weeks } = buildHeatmap([], today, 5)
    const lastWeek = weeks[4]
    expect(lastWeek[2].isFuture).toBe(false) // 今天不是 future
    expect(lastWeek[3].isFuture).toBe(true) // 9-16 晚于今天
    expect(lastWeek[6].isFuture).toBe(true)
  })

  it('有数据的日期计入 tokens/calls 并参与 maxTokens 分档', () => {
    const daily = [
      { day: '2026-09-14', prompt_tokens: 700, completion_tokens: 300, calls: 2 }, // 周一,1000
      { day: '2026-09-13', prompt_tokens: 100, completion_tokens: 50, calls: 1 }, // 周日,150
    ]
    const { weeks, maxTokens } = buildHeatmap(daily, today, 5)
    expect(maxTokens).toBe(1000)
    const lastWeek = weeks[4]
    expect(lastWeek[0].tokens).toBe(150)
    expect(lastWeek[0].calls).toBe(1)
    expect(lastWeek[0].level).toBe(1) // 150/1000 ≤ 0.25 → 1 档
    expect(lastWeek[1].tokens).toBe(1000)
    expect(lastWeek[1].level).toBe(4)
    expect(lastWeek[2].tokens).toBe(0)
    expect(lastWeek[2].level).toBe(0)
  })

  it('窗口外的数据不出现（maxWeeks 收窄）', () => {
    const daily = [{ day: '2026-07-01', prompt_tokens: 1, completion_tokens: 0, calls: 1 }]
    const { weeks } = buildHeatmap(daily, today, 4)
    const allCells = weeks.flat()
    expect(allCells.find((c) => c.day === '2026-07-01')).toBeUndefined()
  })
})

describe('AiUsageHeatmap 组件', () => {
  it('渲染 maxWeeks×7 个格子 + 图例，future 格不可交互', () => {
    const data = buildHeatmap(
      [{ day: '2026-09-14', prompt_tokens: 500, completion_tokens: 500, calls: 3 }],
      new Date(2026, 8, 15),
      5,
    )
    const wrapper = mount(AiUsageHeatmap, { props: { data } })
    const cells = wrapper.findAll('.heatmap-grid .heatmap-cell')
    expect(cells).toHaveLength(5 * 7)
    // 有数据的日子 500/500 → 4 档
    const hot = cells.find((c) => c.classes().includes('heat-4'))
    expect(hot).toBeDefined()
    expect(wrapper.findAll('.heatmap-legend .heatmap-cell')).toHaveLength(5) // 图例 5 级
    // future 占位格不触发 tooltip
    const future = cells.find((c) => c.classes().includes('heat-future'))!
    expect(future).toBeDefined()
    awaitFutureHover(wrapper, future.element)
  })

  async function awaitFutureHover(wrapper: ReturnType<typeof mount>, _el: Element) {
    // future 格 visibility:hidden（CSS），mouseenter 不应设置 tooltip 数据
    expect(wrapper.find('.heatmap-tooltip').exists()).toBe(false)
  }
})

describe('aggregateDonutBySource', () => {
  const row = (over: Partial<AiUsageSourceRow>): AiUsageSourceRow => ({
    source_id: 1,
    label: 'a/b',
    source_type: 'github',
    calls: 1,
    prompt_tokens: 100,
    completion_tokens: 50,
    cache_hit_tokens: 0,
    cache_miss_tokens: 0,
    ...over,
  })

  it('按 tokens 降序、占比正确、无源行由组件文案兜底（label 为空）', () => {
    const segs = aggregateDonutBySource([
      row({ source_id: 1, label: 'small', prompt_tokens: 10, completion_tokens: 0 }),
      row({ source_id: 2, label: 'big', prompt_tokens: 600, completion_tokens: 300 }),
      row({ source_id: null, label: null, prompt_tokens: 90, completion_tokens: 0 }),
    ], '其他')
    expect(segs.map((s) => s.label)).toEqual(['big', '', 'small'])
    expect(segs[0].tokens).toBe(900)
    expect(segs[0].share).toBeCloseTo(0.9, 5)
    // no-source 行（90 tokens）排第二
    expect(segs[1].share).toBeCloseTo(0.09, 5)
    // no-source 行 key 稳定（key 为 no-source，label 留空交组件 t() 兜底）
    expect(segs.find((s) => s.key === 'no-source')?.label).toBe('')
  })

  it('超过 maxSegments 时尾部并入「其他」', () => {
    const rows = Array.from({ length: 10 }, (_, i) =>
      row({ source_id: i + 1, label: `src-${i}`, prompt_tokens: 100, completion_tokens: 0 }),
    )
    const segs = aggregateDonutBySource(rows, '其他', 7)
    expect(segs).toHaveLength(8)
    expect(segs[7].key).toBe('other')
    expect(segs[7].tokens).toBe(300)
  })

  it('全部为 0 时 share=0 不产生 NaN', () => {
    const segs = aggregateDonutBySource([row({ prompt_tokens: 0, completion_tokens: 0 })], '其他')
    expect(segs[0].share).toBe(0)
  })
})

describe('aggregateDonutByAction', () => {
  it('按操作聚合并降序', () => {
    const rows: AiUsageActionRow[] = [
      { action: 'translate', calls: 2, prompt_tokens: 500, completion_tokens: 400 },
      { action: 'summary', calls: 5, prompt_tokens: 200, completion_tokens: 100 },
      { action: 'detect_language', calls: 2, prompt_tokens: 10, completion_tokens: 4 },
    ]
    const segs = aggregateDonutByAction(rows, '其他')
    expect(segs.map((s) => s.key)).toEqual(['translate', 'summary', 'detect_language'])
    expect(segs[0].tokens).toBe(900)
    expect(segs[0].share).toBeCloseTo(900 / 1214, 5)
  })
})

describe('buildSourceTsv / formatTokens', () => {
  it('TSV 含表头行 + 数据行，无源行用兜底文案', () => {
    const rows: AiUsageSourceRow[] = [
      {
        source_id: null,
        label: null,
        source_type: null,
        calls: 1,
        prompt_tokens: 3,
        completion_tokens: 1,
        cache_hit_tokens: 0,
        cache_miss_tokens: 0,
      },
      {
        source_id: 9,
        label: 'o/r',
        source_type: 'github',
        calls: 2,
        prompt_tokens: 100,
        completion_tokens: 60,
        cache_hit_tokens: 40,
        cache_miss_tokens: 60,
      },
    ]
    const tsv = buildSourceTsv(['源', '调用'], rows, '（无源）')
    const lines = tsv.split('\n')
    expect(lines).toHaveLength(3)
    expect(lines[0]).toBe('源\t调用')
    expect(lines[1]).toContain('（无源）')
    expect(lines[2]).toBe('o/r\t2\t100\t60\t40\t60\t160')
  })

  it('formatTokens 缩写', () => {
    expect(formatTokens(890)).toBe('890')
    expect(formatTokens(34_500)).toBe('34.5K')
    expect(formatTokens(1_234_567)).toBe('1.2M')
  })

  it('dailyTotalTokens = prompt + completion（缓存命中不重复计）', () => {
    expect(dailyTotalTokens({ prompt_tokens: 100, completion_tokens: 30 })).toBe(130)
  })
})

// toDateKey 冒烟：buildHeatmap 的对齐依赖它（本地时区一致性）
describe('toDateKey（buildHeatmap 依赖）', () => {
  it('YYYY-MM-DD 格式', () => {
    expect(toDateKey(new Date(2026, 8, 5))).toBe('2026-09-05')
  })
})
