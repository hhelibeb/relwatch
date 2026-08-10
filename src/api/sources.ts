import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { Source } from '../bindings'

// 输入解析与源类型注册表从 source-registry.ts re-export，保持历史 import 路径兼容。
export {
  parseSourceUrl,
  parseGitHubUrl,
  parseHFOrgUrl,
  parseYoutubeUrl,
  parseBilibiliUrl,
  getSourceTypeDef,
  sourceTypeDefs,
  sourceRepoKey,
  sourceDisplayName,
  sourceSearchQuery,
} from './source-registry'
export type { ParsedSource, SourceTypeDef, HfMetaView } from './source-registry'

// 类型由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { Source } from '../bindings'

/**
 * 构造订阅内容类型 config JSON（YouTube 源专用）。
 * @param videos 订阅视频
 * @param live 订阅直播
 * @param posts 订阅帖子（当前数据源不支持，恒为 false）
 */
export function buildYoutubeConfig(videos: boolean, live: boolean, posts = false): string {
  return JSON.stringify({ videos, live, posts })
}

export async function addSource(sourceType: string, owner: string, repo: string, config?: string): Promise<number> {
  return invokeI18nFn(() => commands.addSource(sourceType, owner, repo, config ?? null))
}

export async function removeSource(id: number): Promise<void> {
  await invokeI18nFn(() => commands.removeSource(id))
}

export async function updateSource(id: number, enabled: boolean, pollIntervalMinutes: number, muted?: boolean, config?: string): Promise<void> {
  await invokeI18nFn(() => commands.updateSource(id, enabled, pollIntervalMinutes, muted ?? null, config ?? null))
}

export async function listSources(): Promise<Source[]> {
  return invokeI18nFn(commands.listSources)
}
