import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'

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
  const match = raw.trim().match(/github\.com\/([^/]+)\/([^/?#]+)/)
  if (!match) return null
  return { owner: match[1], repo: match[2] }
}

export async function addSource(sourceType: string, owner: string, repo: string): Promise<number> {
  return invoke<number>('add_source', { sourceType, owner, repo })
}

export async function removeSource(id: number): Promise<void> {
  return invoke('remove_source', { id })
}

export async function updateSource(id: number, enabled: boolean, pollIntervalMinutes: number): Promise<void> {
  return invoke('update_source', { id, enabled, pollIntervalMinutes })
}

export async function listSources(): Promise<Source[]> {
  return invoke<Source[]>('list_sources')
}

export async function getReleases(): Promise<ReleaseInfo[]> {
  return invoke<ReleaseInfo[]>('get_releases')
}

export async function getPendingReleases(): Promise<ReleaseInfo[]> {
  return invoke<ReleaseInfo[]>('get_pending_releases')
}

export async function setNotificationState(
  releaseId: number,
  status: string,
  snoozeMinutes?: number
): Promise<void> {
  return invoke('set_notification_state', { releaseId, status, snoozeMinutes })
}

export async function getLogs(limit: number): Promise<LogEntry[]> {
  return invoke<LogEntry[]>('get_logs', { limit })
}

export async function clearLogs(): Promise<void> {
  return invoke('clear_logs')
}

export async function triggerPoll(): Promise<PollResult> {
  return invoke<PollResult>('trigger_poll')
}

export async function checkSingleSource(id: number): Promise<PollResult> {
  return invoke<PollResult>('check_single_source', { id })
}

export async function getPollCountdown(): Promise<number> {
  return invoke<number>('get_poll_countdown')
}

export async function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>('get_settings')
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
  return invoke('update_settings', { payload: { pollIntervalMinutes, proxyUrl, minimizeToTray, logRetentionDays, deepseekEnabled, deepseekModel, deepseekBaseUrl, deepseekProxyEnabled, checkPrereleases, language } })
}

export async function setDeepseekApiKey(apiKey: string): Promise<void> {
  return invoke('set_deepseek_api_key', { apiKey })
}

export async function setGithubToken(token: string): Promise<void> {
  return invoke('set_github_token', { token })
}

export async function testDeepseekConnection(): Promise<string> {
  return invoke<string>('test_deepseek_connection')
}

export async function openReleaseUrl(url: string): Promise<void> {
  await openUrl(url)
}
