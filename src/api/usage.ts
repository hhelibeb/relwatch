import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { UsageStatRow } from '../bindings'

// 类型由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { UsageDaily, UsageStatRow } from '../bindings'

/** 批量上报点击计数（前端 5s 节流聚合后调用）。 */
export async function recordUsage(events: [string, number][]): Promise<void> {
  if (events.length === 0) return
  await invokeI18nFn(() => commands.recordUsage(events))
}

/** 查询统计（按累计次数降序）。仅开发者工具调用，不进入任何用户 UI。 */
export async function getUsageStats(days?: number): Promise<UsageStatRow[]> {
  return invokeI18nFn(() => commands.getUsageStats(days ?? null))
}

/** 清空全部统计。 */
export async function clearUsageStats(): Promise<void> {
  await invokeI18nFn(commands.clearUsageStats)
}
