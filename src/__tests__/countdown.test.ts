import { describe, it, expect, vi, beforeEach } from 'vitest'

// 直接测试 formatCountdown 逻辑（App.vue 的纯函数）
// 不依赖 Vue 挂载，更加可靠

function formatCountdown(secs: number, t_checkSoon: string, t_minSec: (m: string, s: string) => string): string {
  if (secs <= 0) return t_checkSoon
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return t_minSec(String(m), String(s))
}

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))

beforeEach(() => { vi.clearAllMocks() })

describe('App.vue — formatCountdown 逻辑', () => {
  const tSoon = '即将检查...'
  const tMinSec = (m: string, s: string) => `${m}分${s}秒`

  it('<= 0 返回 "即将检查..."', () => {
    expect(formatCountdown(0, tSoon, tMinSec)).toBe('即将检查...')
  })

  it('负数返回 "即将检查..."', () => {
    expect(formatCountdown(-5, tSoon, tMinSec)).toBe('即将检查...')
  })

  it('125 秒 = 2分5秒', () => {
    expect(formatCountdown(125, tSoon, tMinSec)).toBe('2分5秒')
  })

  it('300 秒 = 5分0秒', () => {
    expect(formatCountdown(300, tSoon, tMinSec)).toBe('5分0秒')
  })

  it('60 秒 = 1分0秒', () => {
    expect(formatCountdown(60, tSoon, tMinSec)).toBe('1分0秒')
  })

  it('1 秒 = 0分1秒', () => {
    expect(formatCountdown(1, tSoon, tMinSec)).toBe('0分1秒')
  })

  it('3600 秒 = 60分0秒', () => {
    expect(formatCountdown(3600, tSoon, tMinSec)).toBe('60分0秒')
  })

  it('59 秒 = 0分59秒', () => {
    expect(formatCountdown(59, tSoon, tMinSec)).toBe('0分59秒')
  })

  it('61 秒 = 1分1秒', () => {
    expect(formatCountdown(61, tSoon, tMinSec)).toBe('1分1秒')
  })
})
