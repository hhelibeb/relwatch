import { describe, it, expect } from 'vitest'
import {
  MEDIA_BASE,
  isRemoteImageUrl,
  toMediaUrl,
  fromMediaUrl,
  mediaUrlOrEmpty,
} from '../utils/imageProxy'

describe('imageProxy', () => {
  describe('isRemoteImageUrl', () => {
    it('识别 http/https 为远程图片', () => {
      expect(isRemoteImageUrl('https://i.ytimg.com/vi/x/mqdefault.jpg')).toBe(true)
      expect(isRemoteImageUrl('http://i0.hdslb.com/bfs/a.jpg')).toBe(true)
    })

    it('data/blob/相对/自身资源不是远程图片', () => {
      expect(isRemoteImageUrl('data:image/png;base64,xxx')).toBe(false)
      expect(isRemoteImageUrl('blob:http://localhost/uuid')).toBe(false)
      expect(isRemoteImageUrl('/icons.svg#x')).toBe(false)
      expect(isRemoteImageUrl('icons.svg')).toBe(false)
    })
  })

  describe('toMediaUrl', () => {
    it('把 https 图片改写为 media 前缀 + 编码', () => {
      const url = 'https://i.ytimg.com/vi/abc/mqdefault.jpg'
      expect(toMediaUrl(url)).toBe(MEDIA_BASE + encodeURIComponent(url))
    })

    it('对 media 前缀幂等（不重复包裹）', () => {
      const once = MEDIA_BASE + encodeURIComponent('https://example.com/a.png')
      expect(toMediaUrl(once)).toBe(once)
    })

    it('data:/相对路径/空值原样返回', () => {
      expect(toMediaUrl('data:image/png;base64,xxx')).toBe('data:image/png;base64,xxx')
      expect(toMediaUrl('/icons.svg#x')).toBe('/icons.svg#x')
      expect(toMediaUrl('')).toBe('')
    })

    it('URL 含查询串整体编码', () => {
      const url = 'https://example.com/a.png?x=1&y=2'
      const out = toMediaUrl(url)
      expect(out.startsWith(MEDIA_BASE)).toBe(true)
      expect(fromMediaUrl(out)).toBe(url)
    })
  })

  describe('fromMediaUrl', () => {
    it('还原 media 地址为原始 URL', () => {
      const url = 'https://i.ytimg.com/vi/abc/mqdefault.jpg'
      expect(fromMediaUrl(MEDIA_BASE + encodeURIComponent(url))).toBe(url)
    })

    it('非 media 前缀原样返回', () => {
      expect(fromMediaUrl('https://example.com/a.png')).toBe('https://example.com/a.png')
    })

    it('损坏编码回退为原值', () => {
      expect(fromMediaUrl(MEDIA_BASE + '%ZZ')).toBe(MEDIA_BASE + '%ZZ')
    })
  })

  describe('mediaUrlOrEmpty', () => {
    it('null/undefined → 空串', () => {
      expect(mediaUrlOrEmpty(null)).toBe('')
      expect(mediaUrlOrEmpty(undefined)).toBe('')
      expect(mediaUrlOrEmpty('')).toBe('')
    })

    it('有值走 toMediaUrl', () => {
      const url = 'https://example.com/x.png'
      expect(mediaUrlOrEmpty(url)).toBe(MEDIA_BASE + encodeURIComponent(url))
    })
  })
})
