import { openUrl } from '@tauri-apps/plugin-opener'
import { commands } from '../bindings'
import { t } from '../i18n'
import { extractErrorMessage } from '../services/feedback'

export function translateError(raw: string): string {
  const msg = raw.replace(/^Error:\s*/, '')
  if (!msg.startsWith('err.')) return msg
  const parts = msg.split('|')
  const key = parts[0]
  const args = parts.slice(1)
  return t(key, ...args)
}

export class InvokeI18nError extends Error {
  constructor(
    public readonly key: string,
    public readonly args: string[],
    message: string,
  ) {
    super(message)
    this.name = 'InvokeI18nError'
  }
}

export async function invokeI18nFn<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn()
  } catch (e: unknown) {
    return throwTranslated(e)
  }
}

function throwTranslated(e: unknown): never {
  const clean = extractErrorMessage(e)
  const msg = translateError(clean)
  if (clean.startsWith('err.')) {
    const parts = clean.split('|')
    const err = new InvokeI18nError(parts[0], parts.slice(1), msg)
    if (e instanceof Error && e.stack) err.stack = e.stack
    throw err
  }
  const err = e instanceof Error ? e : new Error(clean)
  err.message = msg
  throw err
}

export async function openReleaseUrl(url: string): Promise<void> {
  await openUrl(url)
}

// 复制文本到剪贴板：统一走 Rust 端写入（navigator.clipboard 依赖文档焦点/用户激活，
// 右键菜单等场景下不可靠；Rust 端已处理 Windows 的线程约束）
export async function copyTextToClipboard(text: string): Promise<void> {
  await invokeI18nFn(() => commands.setClipboardText(text))
}

// 复制图片到剪贴板：
// 1) 经 Rust 端下载字节（绕过 webview CORS，自动继承应用代理设置）
// 2) 用 canvas 统一转码为 PNG（gif/jpg/webp/svg 等格式归一化；blob: URL 同源，canvas 不会被污染）
// 3) PNG 字节交回 Rust 端解码为 RGBA 并写入系统剪贴板
export async function copyImageToClipboard(url: string): Promise<void> {
  const bytes = await invokeI18nFn(() => commands.fetchUrlBytes(url))
  const png = await normalizeToPng(new Uint8Array(bytes))
  await invokeI18nFn(() => commands.setClipboardImage([...png]))
}

async function normalizeToPng(bytes: Uint8Array<ArrayBuffer>): Promise<Uint8Array> {
  const blobUrl = URL.createObjectURL(new Blob([bytes]))
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const el = new Image()
      el.onload = () => resolve(el)
      el.onerror = () => reject(new Error('image decode failed'))
      el.src = blobUrl
    })
    // 无内在尺寸的 SVG 等会拿到 0 尺寸，canvas 无法处理
    if (img.naturalWidth === 0 || img.naturalHeight === 0) {
      throw new Error('image has no intrinsic size')
    }
    const canvas = document.createElement('canvas')
    canvas.width = img.naturalWidth
    canvas.height = img.naturalHeight
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('canvas 2d unsupported')
    ctx.drawImage(img, 0, 0)
    const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/png'))
    if (!blob) throw new Error('png encode failed')
    return new Uint8Array(await blob.arrayBuffer())
  } finally {
    URL.revokeObjectURL(blobUrl)
  }
}
