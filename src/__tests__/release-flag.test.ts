import { describe, it, expect } from 'vitest'
import {
  FLAG_MAX,
  FLAG_COLOR_LABEL_KEYS,
  isFlagged,
  releaseFlagged,
  releaseFlagColor,
  flagColorByIndex,
} from '../utils/releaseFlag'
import { createRelease } from './helpers'
import type { ReleaseInfo } from '../api/releases'

/**
 * releaseFlag.ts 旗标展示规则（纯函数）
 *
 * 旗标为单选颜色：0 = 未标记，1-6 = 红/橙/黄/绿/蓝/紫；
 * 越界值（<0 或 >6）按未标记处理，颜色一律经 CSS var 注入。
 */
describe('releaseFlag — isFlagged 边界', () => {
  it('0 为未标记，1-6 为已标记', () => {
    expect(isFlagged(0)).toBe(false)
    for (let i = 1; i <= 6; i++) expect(isFlagged(i)).toBe(true)
  })

  it('越界值（负数、超过 FLAG_MAX）为未标记', () => {
    expect(isFlagged(-1)).toBe(false)
    expect(isFlagged(7)).toBe(false)
    expect(isFlagged(100)).toBe(false)
  })

  it('FLAG_MAX 为 6，label key 数量与颜色编号一一对应', () => {
    expect(FLAG_MAX).toBe(6)
    expect(FLAG_COLOR_LABEL_KEYS).toHaveLength(FLAG_MAX)
  })
})

describe('releaseFlag — releaseFlagged / releaseFlagColor', () => {
  it('releaseFlagged 跟随 release.flag 判定', () => {
    expect(releaseFlagged(createRelease({ flag: 0 }) as ReleaseInfo)).toBe(false)
    expect(releaseFlagged(createRelease({ flag: 4 }) as ReleaseInfo)).toBe(true)
  })

  it('releaseFlagColor 返回对应色板 CSS var，未标记返回 null', () => {
    expect(releaseFlagColor(createRelease({ flag: 1 }) as ReleaseInfo)).toBe('var(--flag-1)')
    expect(releaseFlagColor(createRelease({ flag: 6 }) as ReleaseInfo)).toBe('var(--flag-6)')
    expect(releaseFlagColor(createRelease({ flag: 0 }) as ReleaseInfo)).toBe(null)
  })
})

describe('releaseFlag — flagColorByIndex', () => {
  it('1-6 逐一返回 var(--flag-N)', () => {
    for (let i = 1; i <= 6; i++) {
      expect(flagColorByIndex(i)).toBe(`var(--flag-${i})`)
    }
  })

  it('越界（0 / 7 / 负数）返回 null', () => {
    expect(flagColorByIndex(0)).toBe(null)
    expect(flagColorByIndex(7)).toBe(null)
    expect(flagColorByIndex(-2)).toBe(null)
  })
})
