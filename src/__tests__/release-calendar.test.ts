import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ReleaseCalendar from '../components/ReleaseCalendar.vue'
import { toDateKey, parseDateKey } from '../utils/dateKey'
import type { ReleaseInfo } from '../api/releases'

// 阶段 2-1：月历网格（ReleaseCalendar.vue）专项测试。
// 覆盖：月历 4-6 行网格、周一起始（zh-CN）/周日起始（en-US）、
// 跨月前后填充、闰年 2 月、12 月跨年、空数据与同日多版本计数。
//
// 注意：monthGrid 的"不满 5 行补到 6 行"是组件既有行为（cells.length < 35
// 时多补 7 格），测试按实际行为钉住，不评判取舍。

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
  getLocale: vi.fn(() => 'zh-CN'),
}))

vi.mock('../composables/useUsageTracking', () => ({
  track: vi.fn(),
}))

import { getLocale } from '../i18n'

interface Cell {
  key: string
  date: number
  count: number
  isCurrentMonth: boolean
  isToday: boolean
  releases: ReleaseInfo[]
}

function createRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'tauri-apps',
    repo: 'tauri',
    tag_name: 'v2.0.0',
    release_name: 'Tauri 2.0 Stable',
    html_url: 'https://github.com/tauri-apps/tauri/releases/tag/v2.0.0',
    published_at: '2025-06-01T00:00:00Z',
    prerelease: false,
    body: null,
    detected_at: '2025-06-01T00:00:00Z',
    notification_status: 'pending',
    snooze_until: null,
    ai_summary: null,
    ai_importance: null,
    body_translated: null,
    extra_metadata: null,
    source_description: null,
    ...overrides,
  }
}

function mountCalendar(releases: ReleaseInfo[], year: number, month: number) {
  return mount(ReleaseCalendar, { props: { releases, year, month } })
}

function grid(wrapper: ReturnType<typeof mountCalendar>): Cell[] {
  return (wrapper.vm as unknown as { monthGrid: Cell[] }).monthGrid
}

beforeEach(() => {
  vi.mocked(getLocale).mockReturnValue('zh-CN')
})

describe('ReleaseCalendar.vue — 网格行数与周起始', () => {
  it('zh-CN（周一始）：2025-06 → 6 行 42 格，前导 6 格为上月最后一周', () => {
    // 2025-06-01 是周日 → 周一始时前导 6 格（5/26 周一 ~ 5/31 周六）
    const wrapper = mountCalendar([], 2025, 6)
    const cells = grid(wrapper)

    expect(cells).toHaveLength(42)
    expect(wrapper.findAll('.calendar-cell')).toHaveLength(42)
    expect(cells.filter(c => c.isCurrentMonth)).toHaveLength(30)
    expect(cells[0].key).toBe('2025-05-26')
    expect(cells[0].date).toBe(26)
    expect(cells[0].isCurrentMonth).toBe(false)
    // 尾部补 7/1~7/6，填满 6 行
    expect(cells[35].key).toBe('2025-06-30')
    expect(cells[36].key).toBe('2025-07-01')
    expect(cells[41].key).toBe('2025-07-06')
  })

  it('en-US（周日始）：2025-06 → 首格即当月 1 日（30 天不足 5 行，补到 6 行）', () => {
    vi.mocked(getLocale).mockReturnValue('en-US')
    const wrapper = mountCalendar([], 2025, 6)
    const cells = grid(wrapper)

    // 6/1 是周日 → 周日始时当月 1 日在首格；30 格 < 35 格 → 组件补满 6 行
    expect(cells[0].key).toBe('2025-06-01')
    expect(cells[0].isCurrentMonth).toBe(true)
    expect(cells).toHaveLength(42)
  })

  it('en-US：2025-05（5/1 周四，31 天）→ 恰好 35 格 5 行，不补 6 行', () => {
    vi.mocked(getLocale).mockReturnValue('en-US')
    const wrapper = mountCalendar([], 2025, 5)
    const cells = grid(wrapper)

    // 31 + 4（周日始，周四 startDow=4）= 35 格整 → 5 行，无尾部填充
    expect(cells).toHaveLength(35)
    expect(cells[0].key).toBe('2025-04-27')
    expect(cells[34].key).toBe('2025-05-31')
  })

  it('周一始：周几头列首格为周一（5/26 前一周），周日始时首列为周日', () => {
    vi.mocked(getLocale).mockReturnValue('en-US')
    const wrapper = mountCalendar([], 2025, 6)
    const cells = grid(wrapper)
    // 周日始：首格 6/1 是周日
    expect(parseDateKey(cells[0].key).getDay()).toBe(0)
  })

  it('zh-CN：2025-02（非闰年，2/1 周六）→ 前导 5 格 + 28 天 + 补 9 格 = 42 格', () => {
    // 33 格不满 5 行 → 组件既有行为：额外补 7 格到 6 行
    const wrapper = mountCalendar([], 2025, 2)
    const cells = grid(wrapper)

    expect(cells).toHaveLength(42)
    expect(cells[0].key).toBe('2025-01-27')
    expect(cells.filter(c => c.isCurrentMonth)).toHaveLength(28)
    expect(cells[41].key).toBe('2025-03-09')
    // 2 月只有 28 天
    expect(cells.some(c => c.key === '2025-02-29')).toBe(false)
  })

  it('zh-CN：2024-02（闰年，2/1 周四）→ 29 天在网格内', () => {
    const wrapper = mountCalendar([], 2024, 2)
    const cells = grid(wrapper)

    expect(cells).toHaveLength(42)
    expect(cells[0].key).toBe('2024-01-29')
    const feb29 = cells.find(c => c.key === '2024-02-29')
    expect(feb29).toBeTruthy()
    expect(feb29!.isCurrentMonth).toBe(true)
  })

  it('zh-CN：2025-12（12/1 周一）→ 尾部填充跨年到 2026-01', () => {
    const wrapper = mountCalendar([], 2025, 12)
    const cells = grid(wrapper)

    expect(cells).toHaveLength(42)
    expect(cells[0].key).toBe('2025-12-01')
    expect(cells[0].isCurrentMonth).toBe(true)
    // 尾部跨年填充
    expect(cells[30].key).toBe('2025-12-31')
    expect(cells[31].key).toBe('2026-01-01')
    expect(cells[41].key).toBe('2026-01-11')
  })
})

describe('ReleaseCalendar.vue — 计数与分组', () => {
  // 按本地时间构造 published_at：分组键按本地时区（toDateKey(new Date(iso))），
  // 用本地时刻避免跨 UTC 午夜的时区 flaky
  function localIso(y: number, m: number, d: number, h = 12): string {
    return new Date(y, m - 1, d, h).toISOString()
  }

  it('同日多条 release 聚合到同一格，count 正确', () => {
    const releases = [
      createRelease({ id: 1, published_at: localIso(2025, 6, 15, 8) }),
      createRelease({ id: 2, published_at: localIso(2025, 6, 15, 20) }),
      createRelease({ id: 3, published_at: localIso(2025, 6, 1, 12) }),
    ]
    const wrapper = mountCalendar(releases, 2025, 6)
    const cells = grid(wrapper)

    const day15 = cells.find(c => c.key === '2025-06-15')
    expect(day15!.count).toBe(2)
    expect(day15!.releases.map(r => r.id)).toEqual([1, 2])
    const day1 = cells.find(c => c.key === '2025-06-01')
    expect(day1!.count).toBe(1)
  })

  it('空数据：所有格子 count=0、releases 为空数组', () => {
    const wrapper = mountCalendar([], 2025, 6)
    const cells = grid(wrapper)

    expect(cells.every(c => c.count === 0)).toBe(true)
    expect(cells.every(c => c.releases.length === 0)).toBe(true)
    // 空数据时格内不渲染 count 徽标
    expect(wrapper.findAll('.cell-count')).toHaveLength(0)
  })

  it('月末边界：release 落在 6/30 计入当月，7 月补位格子为空', () => {
    const releases = [createRelease({ id: 9, published_at: localIso(2025, 6, 30, 12) })]
    const wrapper = mountCalendar(releases, 2025, 6)
    const cells = grid(wrapper)

    expect(cells.find(c => c.key === '2025-06-30')!.count).toBe(1)
    expect(cells.find(c => c.key === '2025-07-01')!.count).toBe(0)
    expect(cells.find(c => c.key === '2025-07-01')!.isCurrentMonth).toBe(false)
  })

  it('今天所在的格子被标记 isToday', () => {
    const wrapper = mountCalendar([], new Date().getFullYear(), new Date().getMonth() + 1)
    const cells = grid(wrapper)
    const todayKey = toDateKey(new Date())
    const todayCell = cells.find(c => c.key === todayKey)
    // 当月必然包含今天（网格覆盖整个当前月）
    expect(todayCell).toBeTruthy()
    expect(todayCell!.isToday).toBe(true)
    // 其它格子不误标
    expect(cells.filter(c => c.isToday)).toHaveLength(1)
  })
})
