import { invoke, type InvokeArgs } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { t } from './i18n'

export function translateError(raw: string): string {
  const msg = raw.replace(/^Error:\s*/, '')
  if (!msg.startsWith('err.')) return raw
  const parts = msg.split('|')
  const key = parts[0]
  const args = parts.slice(1)
  return t(key, ...args)
}

async function invokeI18n<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(cmd, args)
  } catch (e: any) {
    const raw = e?.message ?? e?.toString?.() ?? String(e)
    const msg = translateError(raw)
    const err: any = new Error(msg)
    err.toString = () => msg
    throw err
  }
}

export interface Source {
  id: number
  source_type: string
  owner: string
  repo: string
  poll_interval_minutes: number
  enabled: boolean
  created_at: string
  updated_at: string
}

export interface ReleaseInfo {
  id: number
  source_id: number
  source_type: string
  owner: string
  repo: string
  tag_name: string
  release_name: string
  html_url: string
  published_at: string
  prerelease: boolean
  body: string | null
  detected_at: string
  notification_status: string
  snooze_until: string | null
  ai_summary: string | null
  ai_importance: string | null
}

export interface LogEntry {
  id: number
  level: string
  message: string
  created_at: string
  message_key: string | null
  message_args: string | null
}

export interface PollResult {
  new_releases: ReleaseInfo[]
}

export interface AppSettings {
  poll_interval_minutes: number
  proxy_url: string
  minimize_to_tray: boolean
  log_retention_days: number
  deepseek_enabled: boolean
  deepseek_model: string
  deepseek_base_url: string
  deepseek_api_key_set: boolean
  deepseek_proxy_enabled: boolean
  check_prereleases: boolean
  language: string
  github_token_set: boolean
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

export async function getReleases(): Promise<ReleaseInfo[]> {
  return invokeI18n<ReleaseInfo[]>('get_releases')
}

export async function getPendingReleases(): Promise<ReleaseInfo[]> {
  return invokeI18n<ReleaseInfo[]>('get_pending_releases')
}

export async function setNotificationState(
  releaseId: number,
  status: string,
  snoozeMinutes?: number
): Promise<void> {
  return invokeI18n('set_notification_state', { releaseId, status, snoozeMinutes })
}

export async function getLogs(limit: number): Promise<LogEntry[]> {
  return invokeI18n<LogEntry[]>('get_logs', { limit })
}

export async function clearLogs(): Promise<void> {
  return invokeI18n('clear_logs')
}

export async function triggerPoll(): Promise<PollResult> {
  return invokeI18n<PollResult>('trigger_poll')
}

export async function checkSingleSource(id: number): Promise<PollResult> {
  return invokeI18n<PollResult>('check_single_source', { id })
}

export async function getPollCountdown(): Promise<number> {
  return invokeI18n<number>('get_poll_countdown')
}

export async function getSettings(): Promise<AppSettings> {
  return invokeI18n<AppSettings>('get_settings')
}

export async function updateSettings(
  pollIntervalMinutes: number,
  proxyUrl: string,
  minimizeToTray: boolean,
  logRetentionDays: number,
  deepseekEnabled: boolean,
  deepseekModel: string,
  deepseekBaseUrl: string,
  deepseekProxyEnabled: boolean,
  checkPrereleases: boolean,
  language: string,
): Promise<void> {
  return invokeI18n('update_settings', { payload: { pollIntervalMinutes, proxyUrl, minimizeToTray, logRetentionDays, deepseekEnabled, deepseekModel, deepseekBaseUrl, deepseekProxyEnabled, checkPrereleases, language } })
}

export async function setDeepseekApiKey(apiKey: string): Promise<void> {
  return invokeI18n('set_deepseek_api_key', { apiKey })
}

export async function setGithubToken(token: string): Promise<void> {
  return invokeI18n('set_github_token', { token })
}

export async function testDeepseekConnection(): Promise<string> {
  return invokeI18n<string>('test_deepseek_connection')
}

export async function openReleaseUrl(url: string): Promise<void> {
  await openUrl(url)
}
