import { invokeI18n } from './client'

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

export interface Source {
  id: number
  source_type: string
  owner: string
  repo: string
  poll_interval_minutes: number
  enabled: boolean
  last_checked_at: string | null
  last_check_status: string
  last_check_message: string | null
  consecutive_failures: number
  last_new_count: number
  muted: boolean
  created_at: string
  updated_at: string
  description: string | null
  /** 源级附加配置（JSON），目前用于 YouTube 订阅内容类型（视频/直播/帖子）。 */
  config: string | null
}

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
  return invokeI18n<number>('add_source', { sourceType, owner, repo, config: config ?? null })
}

export async function removeSource(id: number): Promise<void> {
  return invokeI18n('remove_source', { id })
}

export async function updateSource(id: number, enabled: boolean, pollIntervalMinutes: number, muted?: boolean, config?: string): Promise<void> {
  const args: Record<string, unknown> = { id, enabled, pollIntervalMinutes }
  if (muted !== undefined) args.muted = muted
  if (config !== undefined) args.config = config
  return invokeI18n('update_source', args)
}

export async function listSources(): Promise<Source[]> {
  return invokeI18n<Source[]>('list_sources')
}
