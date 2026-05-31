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
