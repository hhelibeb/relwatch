import { describe, it, expect } from 'vitest'
import { formatCompactCount, formatVideoDuration, formatViewCount, parseVideoMeta } from '../utils/releaseMeta'

describe('releaseMeta', () => {
  it('解析视频元数据并将 http 封面升级为 https', () => {
    expect(parseVideoMeta(null)).toBeNull()
    expect(parseVideoMeta('{bad json')).toBeNull()
    expect(parseVideoMeta(JSON.stringify({
      thumbnail: 'http://i0.hdslb.com/cover.jpg',
      kind: 'live',
      duration: 'PT1H2M3S',
      view_count: 12345,
    }))).toEqual({
      thumbnail: 'https://i0.hdslb.com/cover.jpg',
      kind: 'live',
      duration: 'PT1H2M3S',
      viewCount: 12345,
    })
  })

  it('格式化 ISO 8601 与冒号时长，非法值返回空串', () => {
    expect(formatVideoDuration(null)).toBe('')
    expect(formatVideoDuration('PT1H2M3S')).toBe('1:02:03')
    expect(formatVideoDuration('PT12M5S')).toBe('12:05')
    expect(formatVideoDuration('PT45S')).toBe('0:45')
    expect(formatVideoDuration('12:34')).toBe('12:34')
    expect(formatVideoDuration('invalid')).toBe('')
  })

  it('按中英文环境格式化播放量并处理舍入边界', () => {
    expect(formatViewCount(9949, 'zh-CN')).toBe('9949')
    expect(formatViewCount(9950, 'zh-CN')).toBe('1万')
    expect(formatViewCount(123400, 'zh-CN')).toBe('12.3万')
    expect(formatViewCount(99_950_000, 'zh-CN')).toBe('1亿')
    expect(formatViewCount(994, 'en-US')).toBe('994')
    expect(formatViewCount(995, 'en-US')).toBe('1K')
    expect(formatViewCount(999_500, 'en-US')).toBe('1M')
  })

  it('格式化紧凑数字', () => {
    expect(formatCompactCount(999)).toBe('999')
    expect(formatCompactCount(1500)).toBe('1.5K')
    expect(formatCompactCount(2_000_000)).toBe('2M')
  })
})
