import { describe, it, expect } from 'vitest'
import { toDateKey, parseDateKey } from '../utils/dateKey'

// 阶段 2-1：日历分组键（src/utils/dateKey.ts）专项测试。
// 日历视图（ReleaseCalendar.vue）以该键做按天分组，闰年/跨月/跨年边界
// 是"最易错边界计算"之一，补上独立测试钉住行为。

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
