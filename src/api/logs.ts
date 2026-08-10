import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { LogSearchResult } from '../bindings'

// 类型由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { LogEntry, LogSearchResult } from '../bindings'

export async function searchLogs(keyword: string, page: number, pageSize: number, level?: string): Promise<LogSearchResult> {
  return invokeI18nFn(() => commands.searchLogs(keyword, page, pageSize, level ?? null))
}

export async function clearLogs(): Promise<void> {
  await invokeI18nFn(commands.clearLogs)
}
