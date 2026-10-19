import { describe, it, expect } from 'vitest'
import { releaseTitleRedundant } from '../utils/releaseDisplay'
import { createRelease } from './helpers'

/**
 * 卡片第二行标题的冗余判定。
 *
 * GitHub 的 release name 惯例是「tag + codename」，与 tag_name 不相等但信息重合；
 * 这里钉住「哪些算重复（不渲染）、哪些必须保留」，避免日后放宽规则吞掉真实信息。
 */
describe('releaseTitleRedundant', () => {
  const redundant = (tag: string, name: string) =>
    releaseTitleRedundant(createRelease({ tag_name: tag, release_name: name }))

  it('codename 引在末尾时视为与版本号重复', () => {
    expect(redundant('v3.6.0', 'v3.6.0 "Vapor"')).toBe(true)
    expect(redundant('v3.6.0', "v3.6.0 'Vapor'")).toBe(true)
    expect(redundant('v3.6.0', 'v3.6.0 “Vapor”')).toBe(true)
    expect(redundant('v3.6.0', 'v3.6.0 「Vapor」')).toBe(true)
  })

  it('仅差 v 前缀或大小写时视为重复', () => {
    expect(redundant('v2.0.0', '2.0.0')).toBe(true)
    expect(redundant('v2.0.0', 'V2.0.0')).toBe(true)
    expect(redundant('15.1.0', '15.1.0')).toBe(true)
  })

  it('release_name 为空或纯空白时视为重复（无处可显示）', () => {
    expect(redundant('v2.0.0', '')).toBe(true)
    expect(redundant('v2.0.0', '   ')).toBe(true)
  })

  it('带版本号之外的信息时不算重复', () => {
    expect(redundant('v2.0.0', 'Tauri 2.0 Stable')).toBe(false)
    expect(redundant('1.2.0', '1.2.0 - Hotfix for #123')).toBe(false)
    expect(redundant('v1.2.0', 'v1.2.0 (2026-09-01)')).toBe(false)
    expect(redundant('v1.2.0', 'Release v1.2.0')).toBe(false)
    expect(redundant('v1.2.0', '1.2.0 Vapor')).toBe(false)
  })

  it('只剥末尾引号内容，不误删中间的引号', () => {
    expect(redundant('v1.0.0', 'v1.0.0 "a" and "b"')).toBe(false)
  })
})
