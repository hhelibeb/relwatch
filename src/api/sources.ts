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

export type ParsedSource =
  | { type: 'github'; owner: string; repo: string }
  | { type: 'huggingface'; owner: string; repo: string }

/** 统一解析输入：优先尝试 GitHub，再尝试 HuggingFace 组织。 */
export function parseSourceUrl(raw: string): ParsedSource | null {
  const input = raw.trim()
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

export async function addSource(sourceType: string, owner: string, repo: string): Promise<number> {
  return invokeI18n<number>('add_source', { sourceType, owner, repo })
}

export async function removeSource(id: number): Promise<void> {
  return invokeI18n('remove_source', { id })
}

export async function updateSource(id: number, enabled: boolean, pollIntervalMinutes: number, muted?: boolean): Promise<void> {
  return invokeI18n('update_source', { id, enabled, pollIntervalMinutes, muted })
}

export async function listSources(): Promise<Source[]> {
  return invokeI18n<Source[]>('list_sources')
}
