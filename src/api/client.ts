import { invoke, type InvokeArgs } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { t } from '../i18n'

export function translateError(raw: string): string {
  const msg = raw.replace(/^Error:\s*/, '')
  if (!msg.startsWith('err.')) return msg
  const parts = msg.split('|')
  const key = parts[0]
  const args = parts.slice(1)
  return t(key, ...args)
}

export async function invokeI18n<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(cmd, args)
  } catch (e: unknown) {
    const raw = e instanceof Error ? e.message : String(e)
    const msg = translateError(raw)
    // 复用原始 Error 以保留调用堆栈
    const err = e instanceof Error ? e : new Error(raw)
    err.message = msg
    throw err
  }
}

export async function openReleaseUrl(url: string): Promise<void> {
  await openUrl(url)
}

// 复制文本到剪贴板：统一走 Rust 端写入（navigator.clipboard 依赖文档焦点/用户激活，
// 右键菜单等场景下不可靠；Rust 端已处理 Windows 的线程约束）
export async function copyTextToClipboard(text: string): Promise<void> {
  await invokeI18n('set_clipboard_text', { text })
}

// 复制图片到剪贴板：
// 1) 经 Rust 端下载字节（绕过 webview CORS，自动继承应用代理设置）
// 2) 用 canvas 统一转码为 PNG（gif/jpg/webp/svg 等格式归一化；blob: URL 同源，canvas 不会被污染）
// 3) PNG 字节交回 Rust 端解码为 RGBA 并写入系统剪贴板
export async function copyImageToClipboard(url: string): Promise<void> {
  const bytes = await invokeI18n<number[]>('fetch_url_bytes', { url })
  const png = await normalizeToPng(new Uint8Array(bytes))
  await invokeI18n('set_clipboard_image', { bytes: png })
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
