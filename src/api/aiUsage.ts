import { invokeI18nFn } from './client'
import { commands } from '../bindings'

// 类型由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type {
  AiUsageStats,
  AiUsageDaily,
  AiUsageSourceRow,
  AiUsageActionRow,
} from '../bindings'
import type { AiUsageStats } from '../bindings'

/** 查询 AI（摘要/翻译/语言检测/连接测试）token 用量聚合。
 *  sourceId 为 null 统计全部源；days 为 null 统计全部历史。 */
export async function getAiUsageStats(sourceId: number | null, days: number | null): Promise<AiUsageStats> {
  return invokeI18nFn(() => commands.getAiUsageStats(sourceId, days))
}
