import { describe, it, expect } from 'vitest'
import { formatCountdown } from '../utils'
import { setLocale } from '../i18n'

// 直接测试 utils.formatCountdown 真实实现（App.vue 通过 import 使用同一函数）
// 不依赖 Vue 挂载，更加可靠
setLocale('zh-CN')

describe('utils.formatCountdown', () => {
  it('<= 0 返回 "即将检查..."', () => {
    expect(formatCountdown(0)).toBe('即将检查...')
  })

  it('负数返回 "即将检查..."', () => {
    expect(formatCountdown(-5)).toBe('即将检查...')
  })

  it('125 秒 = 2分5秒', () => {
    expect(formatCountdown(125)).toBe('2分5秒')
  })

  it('300 秒 = 5分0秒', () => {
    expect(formatCountdown(300)).toBe('5分0秒')
  })

  it('60 秒 = 1分0秒', () => {
    expect(formatCountdown(60)).toBe('1分0秒')
  })

  it('1 秒 = 0分1秒', () => {
    expect(formatCountdown(1)).toBe('0分1秒')
  })

  it('3600 秒 = 60分0秒', () => {
    expect(formatCountdown(3600)).toBe('60分0秒')
  })

  it('59 秒 = 0分59秒', () => {
    expect(formatCountdown(59)).toBe('0分59秒')
  })

  it('61 秒 = 1分1秒', () => {
    expect(formatCountdown(61)).toBe('1分1秒')
  })
})
