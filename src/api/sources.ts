import { invokeI18n } from './client'

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

export function parseGitHubUrl(raw: string): { owner: string; repo: string } | null {
  const input = raw.trim()
  const urlMatch = input.match(/github\.com\/([^/]+)\/([^/?#]+)/)
  if (urlMatch) return { owner: urlMatch[1], repo: urlMatch[2] }
  if (input.includes('github.com')) return null
  const repoMatch = input.match(/^([a-zA-Z0-9][a-zA-Z0-9_.-]*)\/([a-zA-Z0-9_.-]+)$/)
  if (repoMatch) return { owner: repoMatch[1], repo: repoMatch[2] }
  return null
}

/** 解析 HuggingFace 组织输入，支持组织名或 huggingface.co/<org> 链接。 */
export function parseHFOrgUrl(raw: string): string | null {
  const input = raw.trim()
  // https://huggingface.co/organizations/moonshotai/ 或带尾随路径
  const orgMatch = input.match(/huggingface\.co\/organizations\/([a-zA-Z0-9_-]+)/)
  if (orgMatch) return orgMatch[1]
  // https://huggingface.co/moonshotai（但排除 /datasets、/spaces 等保留路径）
  const urlMatch = input.match(/huggingface\.co\/([a-zA-Z0-9_-]+)/)
  if (urlMatch && !['datasets', 'spaces', 'models', 'org', 'organizations', 'settings', 'login'].includes(urlMatch[1])) {
    return urlMatch[1]
  }
  if (input.includes('huggingface.co')) return null
  // 已经是组织名
  if (/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(input)) return input
  return null
}

/** 解析 YouTube 频道输入，返回 owner（channel_id 或 @handle）。 */
export function parseYoutubeUrl(raw: string): string | null {
  let input = raw.trim()
  // 解码 URL 编码（如 @%E6%81%8B%E4%B8%8A%E9%BB%98%E7%99%BD → @恋上默白）
  try {
    if (input.includes('%')) input = decodeURIComponent(input)
  } catch {
    // 非法转义序列时保持原样，由后续规则判定
  }
  // 已是 channel_id（UC + 22 位 base64 字符）
  if (/^UC[a-zA-Z0-9_-]{10,}$/.test(input)) return input
  // 以 UC 开头但长度不足：不是合法 channel_id，也不应视为 handle
  if (/^UC/.test(input)) return null
  // 纯 handle（@xxx 或 xxx，支持中文等 Unicode 字符）
  if (/^@?[\p{L}\p{N}_.-]{3,30}$/u.test(input)) {
    return input.startsWith('@') ? input : `@${input}`
  }
  // https://www.youtube.com/channel/UCxxx / youtube.com/channel/UCxxx
  const channelMatch = input.match(/(?:youtube\.com)\/channel\/(UC[a-zA-Z0-9_-]+)/)
  if (channelMatch) return channelMatch[1]
  // @handle 链接（youtube.com/@handle，支持 Unicode）
  const linkMatch = input.match(/(?:youtube\.com)\/@([^/?#]{1,50})/)
  if (linkMatch) return `@${linkMatch[1]}`
  const customMatch = input.match(/(?:youtube\.com)\/(?:c|user)\/([^/?#]{1,50})/)
  if (customMatch) return `@${customMatch[1]}`
  return null
}

export type ParsedSource =
  | { type: 'github'; owner: string; repo: string }
  | { type: 'huggingface'; owner: string; repo: string }
  | { type: 'youtube'; owner: string; repo: string }

/** 统一解析输入：YouTube → GitHub → HuggingFace 组织。 */
export function parseSourceUrl(raw: string): ParsedSource | null {
  const input = raw.trim()
  // YouTube：链接、@handle、channel_id 优先识别
  if (
    input.includes('youtube.com') ||
    input.includes('youtu.be') ||
    input.startsWith('@') ||
    /^UC[a-zA-Z0-9_-]{10,}$/.test(input)
  ) {
    const owner = parseYoutubeUrl(input)
    return owner ? { type: 'youtube', owner, repo: '' } : null
  }
  // 明确的 huggingface 链接走 HF 分支
  if (input.includes('huggingface.co')) {
    const org = parseHFOrgUrl(input)
    return org ? { type: 'huggingface', owner: org, repo: '' } : null
  }
  const gh = parseGitHubUrl(input)
  if (gh) return { type: 'github', owner: gh.owner, repo: gh.repo }
  // 既非 github 链接/owner-repo，也非明确 HF 链接：尝试当作 HF 组织名
  const org = parseHFOrgUrl(input)
  return org ? { type: 'huggingface', owner: org, repo: '' } : null
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
