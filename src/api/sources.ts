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

export async function addSource(sourceType: string, owner: string, repo: string): Promise<number> {
  return invokeI18n<number>('add_source', { sourceType, owner, repo })
}

export async function removeSource(id: number): Promise<void> {
  return invokeI18n('remove_source', { id })
}

export async function updateSource(id: number, enabled: boolean, pollIntervalMinutes: number): Promise<void> {
  return invokeI18n('update_source', { id, enabled, pollIntervalMinutes })
}

export async function listSources(): Promise<Source[]> {
  return invokeI18n<Source[]>('list_sources')
}
