import { describe, it, expect, vi, afterEach } from 'vitest'
import { toDateKey, parseDateKey, CALENDAR_MIN_YEAR, calendarMonthAhead, clampCalendarMonth } from '../utils/dateKey'

// 日历分组键（src/utils/dateKey.ts）专项测试。
// 日历视图（ReleaseCalendar.vue）以该键做按天分组，闰年/跨月/跨年边界
// 是"最易错边界计算"之一，用独立测试钉住行为。

describe('toDateKey', () => {
  it('常规日期格式化为 YYYY-MM-DD', () => {
    expect(toDateKey(new Date(2025, 5, 15))).toBe('2025-06-15')
  })

  it('月/日补零（个位数补前导 0）', () => {
    expect(toDateKey(new Date(2025, 0, 1))).toBe('2025-01-01')
    expect(toDateKey(new Date(2025, 11, 9))).toBe('2025-12-09')
  })

  it('闰年 2 月 29 日', () => {
    expect(toDateKey(new Date(2024, 1, 29))).toBe('2024-02-29')
  })

  it('12 月 31 日跨年', () => {
    expect(toDateKey(new Date(2025, 11, 31))).toBe('2025-12-31')
  })

  it('非闰年 2 月无 29 日（3 月 1 日）', () => {
    // 2025 非闰年：2/29 溢出到 3/1
    expect(toDateKey(new Date(2025, 1, 29))).toBe('2025-03-01')
  })
})

describe('parseDateKey', () => {
  it('解析 YYYY-MM-DD 为本地时间 Date', () => {
    const d = parseDateKey('2025-06-15')
    expect(d.getFullYear()).toBe(2025)
    expect(d.getMonth()).toBe(5)
    expect(d.getDate()).toBe(15)
  })

  it('解析后与 toDateKey 往返一致', () => {
    for (const key of ['2024-02-29', '2025-01-01', '2025-12-31', '2025-06-15']) {
      expect(toDateKey(parseDateKey(key))).toBe(key)
    }
  })

  it('边界：年初 / 年末 / 闰年', () => {
    expect(toDateKey(parseDateKey('2025-01-01'))).toBe('2025-01-01')
    expect(toDateKey(parseDateKey('2024-12-31'))).toBe('2024-12-31')
    expect(toDateKey(parseDateKey('2024-02-29'))).toBe('2024-02-29')
    expect(toDateKey(parseDateKey('2020-02-29'))).toBe('2020-02-29')
  })

  it('跨月解析：月末 +1 天进入下月', () => {
    const d = parseDateKey('2025-01-31')
    d.setDate(d.getDate() + 1)
    expect(toDateKey(d)).toBe('2025-02-01')
  })
})

// 日历翻页/跳转的月份边界（ReleaseCalendar + ReleaseTab 共用同一套口径）
describe('calendarMonthAhead / clampCalendarMonth', () => {
  afterEach(() => vi.useRealTimers())

  function freezeNow(y: number, m: number) {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(y, m - 1, 15, 12))
  }

  it('calendarMonthAhead：晚于当前月为 true，当月/更早为 false', () => {
    freezeNow(2026, 3)
    expect(calendarMonthAhead(2026, 4)).toBe(true)
    expect(calendarMonthAhead(2027, 1)).toBe(true)
    expect(calendarMonthAhead(2026, 3)).toBe(false)
    expect(calendarMonthAhead(2026, 2)).toBe(false)
    expect(calendarMonthAhead(2025, 12)).toBe(false)
  })

  it('clamp：跨年方向自动进退位（12 月 +1 → 次年 1 月，1 月 -1 → 上年 12 月）', () => {
    freezeNow(2026, 3)
    expect(clampCalendarMonth(2025, 13)).toEqual({ year: 2026, month: 1 })
    expect(clampCalendarMonth(2026, 0)).toEqual({ year: 2025, month: 12 })
  })

  it('clamp：上限封顶到当前月', () => {
    freezeNow(2026, 3)
    expect(clampCalendarMonth(2026, 9)).toEqual({ year: 2026, month: 3 })
    expect(clampCalendarMonth(2030, 1)).toEqual({ year: 2026, month: 3 })
  })

  it('clamp：下限不早于 CALENDAR_MIN_YEAR-01', () => {
    freezeNow(2026, 3)
    expect(clampCalendarMonth(CALENDAR_MIN_YEAR, 0)).toEqual({ year: CALENDAR_MIN_YEAR, month: 1 })
    expect(clampCalendarMonth(1999, 7)).toEqual({ year: CALENDAR_MIN_YEAR, month: 1 })
  })

  it('clamp：合法值原样返回', () => {
    freezeNow(2026, 3)
    expect(clampCalendarMonth(2024, 11)).toEqual({ year: 2024, month: 11 })
  })
})
