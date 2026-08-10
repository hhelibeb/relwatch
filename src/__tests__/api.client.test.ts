import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: vi.fn(),
}))
vi.mock('../i18n', () => ({
  t: vi.fn((key: string, ...args: string[]) => (args.length ? `${key}|${args.join('|')}` : key)),
}))

import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import {
  invokeI18n,
  InvokeI18nError,
  openReleaseUrl,
  copyTextToClipboard,
  copyImageToClipboard,
} from '../api/client'

// ── 环境 stub：Image / canvas / URL.createObjectURL ──────────────

class MockImage {
  static failLoad = false
  static zeroSize = false
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  naturalWidth = 1
  naturalHeight = 1
  set src(_v: string) {
    if (MockImage.failLoad) {
      this.onerror?.()
    } else {
      if (MockImage.zeroSize) {
        this.naturalWidth = 0
        this.naturalHeight = 0
      }
      this.onload?.()
    }
  }
}

const originalImage = globalThis.Image
const originalCreateURL = URL.createObjectURL
const originalRevokeURL = URL.revokeObjectURL

beforeEach(() => {
  vi.clearAllMocks()
  MockImage.failLoad = false
  MockImage.zeroSize = false
  vi.stubGlobal('Image', MockImage)
  // canvas：jsdom 无 2d 实现，直接覆盖 prototype 方法
  const proto = HTMLCanvasElement.prototype as unknown as Record<string, unknown>
  proto.getContext = vi.fn(() => mockCtx)
  proto.toBlob = vi.fn((cb: BlobCallback) => {
    cb(new Blob(['png-bytes'], { type: 'image/png' }))
  })
  URL.createObjectURL = vi.fn(() => 'blob:mock') as unknown as typeof URL.createObjectURL
  URL.revokeObjectURL = vi.fn() as unknown as typeof URL.revokeObjectURL
})

afterEach(() => {
  vi.unstubAllGlobals()
  globalThis.Image = originalImage
  URL.createObjectURL = originalCreateURL
  URL.revokeObjectURL = originalRevokeURL
})

const mockCtx = { drawImage: vi.fn() }

// ── invokeI18n ───────────────────────────────────────────────────

describe('invokeI18n', () => {
  it('成功时返回 invoke 结果', async () => {
    vi.mocked(invoke).mockResolvedValue({ ok: 1 })

    const result = await invokeI18n<{ ok: number }>('some_command', { a: 1 })

    expect(invoke).toHaveBeenCalledWith('some_command', { a: 1 })
    expect(result).toEqual({ ok: 1 })
  })

  it('无参数命令只传命令名', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await invokeI18n('some_command')

    expect(invoke).toHaveBeenCalledWith('some_command', undefined)
  })

  it('Error 且带 err. 前缀时抛 InvokeI18nError（携带原始 key/args，翻译消息并保留堆栈）', async () => {
    const original = new Error('Error: err.add_failed|microsoft/vscode')
    vi.mocked(invoke).mockRejectedValue(original)

    await expect(invokeI18n('some_command')).rejects.toThrow('err.add_failed|microsoft/vscode')
    // 抛 InvokeI18nError：message 已翻译，且携带原始错误 key 与参数供调用方分支判断
    const err = (await invokeI18n('some_command').catch(e => e)) as InvokeI18nError
    expect(err).toBeInstanceOf(InvokeI18nError)
    expect(err.key).toBe('err.add_failed')
    expect(err.args).toEqual(['microsoft/vscode'])
    // 保留原始错误调用堆栈
    expect(err.stack).toBe(original.stack)
  })

  it('Error 无 err. 前缀时不调用 t()，仅去前缀', async () => {
    const { t } = await import('../i18n')
    vi.mocked(invoke).mockRejectedValue(new Error('Error: network timeout'))

    await expect(invokeI18n('some_command')).rejects.toThrow('network timeout')
    expect(t).not.toHaveBeenCalled()
  })

  it('非 Error 抛出值转换为 Error', async () => {
    vi.mocked(invoke).mockRejectedValue('raw string')

    await expect(invokeI18n('some_command')).rejects.toThrow('raw string')
  })
})

// ── openReleaseUrl / copyTextToClipboard ─────────────────────────

describe('openReleaseUrl', () => {
  it('委托 openUrl 打开链接', async () => {
    vi.mocked(openUrl).mockResolvedValue(undefined)

    await openReleaseUrl('https://github.com/x/y/releases/tag/v1')

    expect(openUrl).toHaveBeenCalledWith('https://github.com/x/y/releases/tag/v1')
  })
})

describe('copyTextToClipboard', () => {
  it('走 Rust 端 set_clipboard_text 命令', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await copyTextToClipboard('hello')

    expect(invoke).toHaveBeenCalledWith('set_clipboard_text', { text: 'hello' })
  })
})

// ── copyImageToClipboard / normalizeToPng ────────────────────────

describe('copyImageToClipboard', () => {
  it('成功链路：下载字节 → canvas 转 PNG → 写入剪贴板', async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce([1, 2, 3, 4]) // fetch_url_bytes
      .mockResolvedValueOnce(undefined) // set_clipboard_image
    const createSpy = vi.mocked(URL.createObjectURL)
    const revokeSpy = vi.mocked(URL.revokeObjectURL)

    await copyImageToClipboard('https://example.com/a.png')

    expect(invoke).toHaveBeenNthCalledWith(1, 'fetch_url_bytes', { url: 'https://example.com/a.png' })
    expect(invoke).toHaveBeenNthCalledWith(2, 'set_clipboard_image', expect.objectContaining({ bytes: expect.any(Array) }))
    expect(createSpy).toHaveBeenCalled()
    expect(revokeSpy).toHaveBeenCalled()
  })

  it('fetch_url_bytes 失败时错误传播', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('Error: err.fetch_failed'))

    await expect(copyImageToClipboard('https://example.com/a.png')).rejects.toThrow('err.fetch_failed')
  })
})

describe('normalizeToPng 错误分支（经 copyImageToClipboard 触发）', () => {
  it('图片解码失败（onerror）抛出 image decode failed 并释放 blob URL', async () => {
    MockImage.failLoad = true
    vi.mocked(invoke).mockResolvedValueOnce([1, 2, 3, 4])

    await expect(copyImageToClipboard('https://example.com/a.png')).rejects.toThrow('image decode failed')
    expect(vi.mocked(URL.revokeObjectURL)).toHaveBeenCalled()
  })

  it('无固有尺寸（SVG 等）抛出 image has no intrinsic size', async () => {
    MockImage.zeroSize = true
    vi.mocked(invoke).mockResolvedValueOnce([1, 2, 3, 4])

    await expect(copyImageToClipboard('https://example.com/a.svg')).rejects.toThrow('image has no intrinsic size')
  })

  it('canvas 2d 不可用时抛出 canvas 2d unsupported', async () => {
    const proto = HTMLCanvasElement.prototype as unknown as Record<string, unknown>
    proto.getContext = vi.fn(() => null)
    vi.mocked(invoke).mockResolvedValueOnce([1, 2, 3, 4])

    await expect(copyImageToClipboard('https://example.com/a.png')).rejects.toThrow('canvas 2d unsupported')
  })

  it('PNG 编码失败（toBlob 返回 null）抛出 png encode failed', async () => {
    const proto = HTMLCanvasElement.prototype as unknown as Record<string, unknown>
    proto.toBlob = vi.fn((cb: BlobCallback) => cb(null))
    vi.mocked(invoke).mockResolvedValueOnce([1, 2, 3, 4])

    await expect(copyImageToClipboard('https://example.com/a.png')).rejects.toThrow('png encode failed')
  })
})
