import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ReleaseCalendar from '../components/ReleaseCalendar.vue'
import { toDateKey } from '../utils/dateKey'
import { t, setLocale } from '../i18n'
import { createRelease } from './helpers'
import { track } from '../composables/useUsageTracking'
import type { ReleaseInfo } from '../api/releases'

// 月历网格（ReleaseCalendar.vue）专项测试（真实 i18n，语言切换走 setLocale）：
// 月历 4-6 行网格、周一起始（zh-CN）/周日起始（en-US）、跨月前后填充、
// 闰年 2 月、12 月跨年、空数据与同日多版本计数。
//
// 注：网格结构通过 DOM 断言（单元格数量/日期文本/current-month class）。

vi.mock('../composables/useUsageTracking', () => ({
  track: vi.fn(),
}))

function mountCalendar(releases: ReleaseInfo[], year: number, month: number) {
  return mount(ReleaseCalendar, { props: { releases, year, month } })
}

/** 从 DOM 重建网格（顺序 = 渲染顺序，等价于 monthGrid 的可观察行为） */
function domGrid(wrapper: ReturnType<typeof mountCalendar>) {
  return wrapper.findAll('.calendar-cell').map(c => ({
    date: Number(c.find('.cell-date').text()),
    isCurrentMonth: c.classes().includes('current-month'),
    isToday: c.classes().includes('today'),
    count: c.find('.cell-count').exists() ? Number(c.find('.cell-count').text()) : 0,
  }))
}

beforeEach(() => {
  setLocale('zh-CN')
})

afterEach(() => {
  setLocale('zh-CN')
})

describe('ReleaseCalendar.vue — 网格行数与周起始', () => {
  it('zh-CN（周一始）：2025-06 → 6 行 42 格，前导 6 格为上月最后一周', () => {
    // 2025-06-01 是周日 → 周一始时前导 6 格（5/26 周一 ~ 5/31 周六）
    const wrapper = mountCalendar([], 2025, 6)
    const cells = domGrid(wrapper)

    expect(cells).toHaveLength(42)
    expect(wrapper.findAll('.calendar-cell')).toHaveLength(42)
    expect(cells.filter(c => c.isCurrentMonth)).toHaveLength(30)
    // 首格为 5/26（other-month）
    expect(cells[0].date).toBe(26)
    expect(cells[0].isCurrentMonth).toBe(false)
    // 尾部补 7/1~7/6，填满 6 行
    expect(cells[35].date).toBe(30)
    expect(cells[35].isCurrentMonth).toBe(true)
    expect(cells[36].date).toBe(1)
    expect(cells[36].isCurrentMonth).toBe(false)
    expect(cells[41].date).toBe(6)
    expect(cells[41].isCurrentMonth).toBe(false)
  })

  it('en-US（周日始）：2025-06 → 首格即当月 1 日（30 天不足 5 行，补到 6 行）', () => {
    setLocale('en-US')
    const wrapper = mountCalendar([], 2025, 6)
    const cells = domGrid(wrapper)

    // 6/1 是周日 → 周日始时当月 1 日在首格；30 格 < 35 格 → 组件补满 6 行
    expect(cells[0].date).toBe(1)
    expect(cells[0].isCurrentMonth).toBe(true)
    expect(cells).toHaveLength(42)
  })

  it('en-US：2025-05（5/1 周四，31 天）→ 恰好 35 格 5 行，不补 6 行', () => {
    setLocale('en-US')
    const wrapper = mountCalendar([], 2025, 5)
    const cells = domGrid(wrapper)

    // 31 + 4（周日始，周四 startDow=4）= 35 格整 → 5 行，无尾部填充
    expect(cells).toHaveLength(35)
    // 首格 4/27（other-month），末格 5/31（current-month）
    expect(cells[0].date).toBe(27)
    expect(cells[0].isCurrentMonth).toBe(false)
    expect(cells[34].date).toBe(31)
    expect(cells[34].isCurrentMonth).toBe(true)
  })

  it('语言切换改变周起始：zh-CN 首列为周一，en-US 首列为周日', () => {
    // zh-CN：2025-06 首格是 5/26（周一，other-month）
    const zhWrapper = mountCalendar([], 2025, 6)
    expect(domGrid(zhWrapper)[0].date).toBe(26)

    // en-US：2025-06 首格是 6/1（周日，current-month）
    setLocale('en-US')
    const enWrapper = mountCalendar([], 2025, 6)
    const enCells = domGrid(enWrapper)
    expect(enCells[0].date).toBe(1)
    expect(enCells[0].isCurrentMonth).toBe(true)
  })

  it('zh-CN：2025-02（非闰年，2/1 周六）→ 前导 5 格 + 28 天 + 补 9 格 = 42 格', () => {
    // 33 格不满 5 行 → 组件既有行为：额外补 7 格到 6 行
    const wrapper = mountCalendar([], 2025, 2)
    const cells = domGrid(wrapper)

    expect(cells).toHaveLength(42)
    expect(cells[0].date).toBe(27)
    expect(cells[0].isCurrentMonth).toBe(false)
    expect(cells.filter(c => c.isCurrentMonth)).toHaveLength(28)
    expect(cells[41].date).toBe(9)
    expect(cells[41].isCurrentMonth).toBe(false)
    // 2 月只有 28 天：不存在 2/29
    expect(cells.some(c => c.date === 29 && c.isCurrentMonth)).toBe(false)
  })

  it('zh-CN：2024-02（闰年，2/1 周四）→ 29 天在网格内', () => {
    const wrapper = mountCalendar([], 2024, 2)
    const cells = domGrid(wrapper)

    expect(cells).toHaveLength(42)
    expect(cells[0].date).toBe(29)
    expect(cells[0].isCurrentMonth).toBe(false)
    const feb29 = cells.find(c => c.date === 29 && c.isCurrentMonth)
    expect(feb29).toBeTruthy()
  })

  it('zh-CN：2025-12（12/1 周一）→ 尾部填充跨年到 2026-01', () => {
    const wrapper = mountCalendar([], 2025, 12)
    const cells = domGrid(wrapper)

    expect(cells).toHaveLength(42)
    expect(cells[0].date).toBe(1)
    expect(cells[0].isCurrentMonth).toBe(true)
    // 尾部跨年填充：12/31 后接 1/1、1/11
    expect(cells[30].date).toBe(31)
    expect(cells[30].isCurrentMonth).toBe(true)
    expect(cells[31].date).toBe(1)
    expect(cells[31].isCurrentMonth).toBe(false)
    expect(cells[41].date).toBe(11)
    expect(cells[41].isCurrentMonth).toBe(false)
  })
})

describe('ReleaseCalendar.vue — 计数与分组', () => {
  // 按本地时间构造 published_at：分组键按本地时区（toDateKey(new Date(iso))），
  // 用本地时刻避免跨 UTC 午夜的时区 flaky
  function localIso(y: number, m: number, d: number, h = 12): string {
    return new Date(y, m - 1, d, h).toISOString()
  }

  it('同日多条 release 聚合到同一格，count 徽标正确', () => {
    const releases = [
      createRelease({ id: 1, published_at: localIso(2025, 6, 15, 8) }),
      createRelease({ id: 2, published_at: localIso(2025, 6, 15, 20) }),
      createRelease({ id: 3, published_at: localIso(2025, 6, 1, 12) }),
    ]
    const wrapper = mountCalendar(releases, 2025, 6)
    const cells = domGrid(wrapper)

    const day15 = cells.find(c => c.date === 15 && c.isCurrentMonth)
    expect(day15!.count).toBe(2)
    const day1 = cells.find(c => c.date === 1 && c.isCurrentMonth)
    expect(day1!.count).toBe(1)
  })

  it('空数据：所有格子 count=0，不渲染 count 徽标', () => {
    const wrapper = mountCalendar([], 2025, 6)

    expect(wrapper.findAll('.cell-count')).toHaveLength(0)
  })

  it('月末边界：release 落在 6/30 计入当月，7 月补位格子为空', () => {
    const releases = [createRelease({ id: 9, published_at: localIso(2025, 6, 30, 12) })]
    const wrapper = mountCalendar(releases, 2025, 6)
    const cells = domGrid(wrapper)

    const jun30 = cells.find(c => c.date === 30 && c.isCurrentMonth)
    expect(jun30!.count).toBe(1)
    // 7/1 补位格为空且非当月
    const jul1 = cells.find(c => c.date === 1 && !c.isCurrentMonth)
    expect(jul1).toBeTruthy()
    expect(jul1!.count).toBe(0)
  })

  it('今天所在的格子被标记 today', () => {
    const wrapper = mountCalendar([], new Date().getFullYear(), new Date().getMonth() + 1)
    const todayKey = toDateKey(new Date())
    const todayDate = Number(todayKey.slice(-2))
    const cells = domGrid(wrapper)

    const todayCell = cells.find(c => c.date === todayDate && c.isCurrentMonth && c.isToday)
    expect(todayCell).toBeTruthy()
    // 其它格子不误标
    expect(cells.filter(c => c.isToday)).toHaveLength(1)
  })

  it('legend 显示真实 i18n 文案', () => {
    const wrapper = mountCalendar([], 2025, 6)
    expect(wrapper.find('.calendar-legend-label').text()).toContain(t('release.calendar_legend'))
  })
})

describe('ReleaseCalendar.vue — 年/月快速跳转', () => {
  // 冻结在 2026-03-15：上限即 2026-03
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 2, 15, 12))
  })
  afterEach(() => {
    vi.useRealTimers()
    document.body.click()
  })

  it('导航条顺序：上一年 / 上一月 / 月份标签 / 下一月 / 下一年 / 回今天', () => {
    const wrapper = mountCalendar([], 2025, 6)
    const buttons = wrapper.findAll('.calendar-nav button')

    expect(buttons).toHaveLength(6)
    expect(buttons[0].text()).toBe('«')
    expect(buttons[1].find('svg').exists()).toBe(true)
    expect(buttons[2].classes()).toContain('calendar-month-label')
    expect(buttons[4].text()).toBe('»')
    expect(buttons[5].text()).toContain(t('release.calendar_today'))
  })

  it('回今天：非当月可用，发出当前年月并上报', async () => {
    const wrapper = mountCalendar([], 2025, 6)
    const today = wrapper.findAll('.calendar-nav button')[5]

    expect(today.attributes('disabled')).toBeUndefined()
    await today.trigger('click')

    expect(wrapper.emitted('jumpTo')).toEqual([[2026, 3]])
    expect(track).toHaveBeenCalledWith('calendar.today')
  })

  it('已在当前月时「回今天」禁用', () => {
    const wrapper = mountCalendar([], 2026, 3)

    expect(wrapper.findAll('.calendar-nav button')[5].attributes('disabled')).toBeDefined()
  })

  it('上/下一年：跨年跳转，触及边界时禁用', async () => {
    const wrapper = mountCalendar([], 2024, 6)
    const [prevYear, , , , nextYear] = wrapper.findAll('.calendar-nav button')

    await prevYear.trigger('click')
    await nextYear.trigger('click')
    expect(wrapper.emitted('jumpTo')).toEqual([[2023, 6], [2025, 6]])

    const atMin = mountCalendar([], 2010, 1)
    expect(atMin.findAll('.calendar-nav button')[0].attributes('disabled')).toBeDefined()

    // 2026-06 已超过当前月（2026-03）：下一年不可用
    const atMax = mountCalendar([], 2026, 1)
    expect(atMax.findAll('.calendar-nav button')[4].attributes('disabled')).toBeDefined()
  })

  it('上一月/下一月按钮在边界处禁用（下限 2010-01，上限当前月）', () => {
    const atMin = mountCalendar([], 2010, 1)
    expect(atMin.findAll('.calendar-nav button')[1].attributes('disabled')).toBeDefined()

    // 显示当前月时，下一页（2026-04）越界
    const atMax = mountCalendar([], 2026, 3)
    expect(atMax.findAll('.calendar-nav button')[3].attributes('disabled')).toBeDefined()

    const mid = mountCalendar([], 2026, 2)
    expect(mid.findAll('.calendar-nav button')[1].attributes('disabled')).toBeUndefined()
    expect(mid.findAll('.calendar-nav button')[3].attributes('disabled')).toBeUndefined()

    // 12 月翻页要进位到次年 1 月：次年 1 月未越界时仍可用
    const dec = mountCalendar([], 2025, 12)
    expect(dec.findAll('.calendar-nav button')[3].attributes('disabled')).toBeUndefined()
  })

  it('跨月后仍以实时月份为基准：「回今天」不会被挂载时的快照钉住', async () => {
    const wrapper = mountCalendar([], 2026, 3)
    // 时间推进到下个月，组件未重新挂载
    vi.setSystemTime(new Date(2026, 3, 2, 12))
    await wrapper.setProps({ year: 2026, month: 2 } as Parameters<typeof wrapper.setProps>[0])

    const today = wrapper.findAll('.calendar-nav button')[5]
    expect(today.attributes('disabled')).toBeUndefined()
    await today.trigger('click')

    expect(wrapper.emitted('jumpTo')).toEqual([[2026, 4]])
  })

  it('跨月后翻到新月：「回今天」随之禁用（与「下一月」的口径一致）', async () => {
    const wrapper = mountCalendar([], 2026, 3)
    vi.setSystemTime(new Date(2026, 3, 2, 12))
    await wrapper.setProps({ year: 2026, month: 4 } as Parameters<typeof wrapper.setProps>[0])

    const buttons = wrapper.findAll('.calendar-nav button')
    expect(buttons[3].attributes('disabled')).toBeDefined()
    expect(buttons[5].attributes('disabled')).toBeDefined()
  })

  it('点月份标签打开选择面板：12 个月 + 年份步进', async () => {
    const wrapper = mountCalendar([], 2025, 6)

    await wrapper.get('.calendar-month-label').trigger('click')

    expect(wrapper.find('.calendar-picker').exists()).toBe(true)
    expect(wrapper.get('.calendar-picker-year span').text()).toBe('2025')
    expect(wrapper.findAll('.calendar-picker-months button')).toHaveLength(12)
    expect(wrapper.findAll('.calendar-picker-months button.selected')).toHaveLength(1)
  })

  it('选择面板点月份：发出 jumpTo 并关闭面板', async () => {
    const wrapper = mountCalendar([], 2025, 6)
    await wrapper.get('.calendar-month-label').trigger('click')

    await wrapper.findAll('.calendar-picker-months button')[10].trigger('click')

    expect(wrapper.emitted('jumpTo')).toEqual([[2025, 11]])
    expect(wrapper.find('.calendar-picker').exists()).toBe(false)
    expect(track).toHaveBeenCalledWith('calendar.jump_month')
  })

  it('选择面板里晚于当前月的月份禁用', async () => {
    const wrapper = mountCalendar([], 2026, 1)
    await wrapper.get('.calendar-month-label').trigger('click')

    const buttons = wrapper.findAll('.calendar-picker-months button')
    expect(buttons.slice(0, 3).map(b => b.attributes('disabled'))).toEqual([undefined, undefined, undefined])
    expect(buttons.slice(4).every(b => b.attributes('disabled') !== undefined)).toBe(true)
  })

  it('选择面板年份步进：可到当前年，不可越过下限', async () => {
    const wrapper = mountCalendar([], 2025, 6)
    await wrapper.get('.calendar-month-label').trigger('click')

    const nextYearBtn = () => wrapper.findAll('.calendar-picker-year button')[1]
    const prevYearBtn = () => wrapper.findAll('.calendar-picker-year button')[0]

    await nextYearBtn().trigger('click')
    expect(wrapper.get('.calendar-picker-year span').text()).toBe('2026')
    expect(nextYearBtn().attributes('disabled')).toBeDefined()

    await prevYearBtn().trigger('click')
    await prevYearBtn().trigger('click')
    expect(wrapper.get('.calendar-picker-year span').text()).toBe('2024')

    const atMin = mountCalendar([], 2010, 6)
    await atMin.get('.calendar-month-label').trigger('click')
    expect(atMin.findAll('.calendar-picker-year button')[0].attributes('disabled')).toBeDefined()
  })

  it('点面板外关闭选择面板', async () => {
    const wrapper = mountCalendar([], 2025, 6)
    await wrapper.get('.calendar-month-label').trigger('click')
    expect(wrapper.find('.calendar-picker').exists()).toBe(true)

    document.body.click()
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.calendar-picker').exists()).toBe(false)
  })

  it('标签键盘 Enter/Space 打开面板，Esc 关闭', async () => {
    const wrapper = mountCalendar([], 2025, 6)
    const label = wrapper.get('.calendar-month-label')

    await label.trigger('keydown', { key: 'Enter' })
    expect(wrapper.find('.calendar-picker').exists()).toBe(true)

    await label.trigger('keydown', { key: 'Escape' })
    expect(wrapper.find('.calendar-picker').exists()).toBe(false)
  })
})
