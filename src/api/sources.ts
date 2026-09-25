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
  syncSourceCapabilities,
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

/**
 * 在已有 config 上打补丁（undefined 表示删除该键）：源级开关与 YouTube 订阅明细
 * 共用 `sources.config` 这一个槽位，所以任何一侧写入都必须**保留另一侧的键**，
 * 否则用户在源设置里改间隔会把订阅勾选清掉（反之亦然）。
 */
export function patchSourceConfig(existing: string | null | undefined, patch: Record<string, unknown>): string {
  let obj: Record<string, unknown> = {}
  if (existing) {
    try {
      const parsed = JSON.parse(existing)
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) obj = parsed as Record<string, unknown>
    } catch {
      // 脏 config（历史手改/旧版本）按空对象处理，由本次写入覆盖成合法 JSON
    }
  }
  for (const [k, v] of Object.entries(patch)) {
    if (v === undefined || v === null) delete obj[k]
    else obj[k] = v
  }
  return JSON.stringify(obj)
}

/** 读取源级开关：未配置返回 undefined，表示「跟随全局设置」。 */
export function readConfigFlag(config: string | null | undefined, key: string): boolean | undefined {
  if (!config) return undefined
  try {
    const parsed = JSON.parse(config)
    const v = parsed?.[key]
    return typeof v === 'boolean' ? v : undefined
  } catch {
    return undefined
  }
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
