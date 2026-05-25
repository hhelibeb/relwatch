import { invokeI18n } from './client'

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
  fetch_history: boolean
  fetch_history_count: number
  language: string
  theme: string
  github_token_set: boolean
}

export interface UpdateSettingsPayload {
  pollIntervalMinutes: number
  proxyUrl: string
  minimizeToTray: boolean
  logRetentionDays: number
  deepseekEnabled: boolean
  deepseekModel: string
  deepseekBaseUrl: string
  deepseekProxyEnabled: boolean
  checkPrereleases: boolean
  fetchHistory: boolean
  fetchHistoryCount: number
  language: string
  theme: string
}

export async function getSettings(): Promise<AppSettings> {
  return invokeI18n<AppSettings>('get_settings')
}

export async function updateSettings(payload: UpdateSettingsPayload): Promise<void> {
  return invokeI18n('update_settings', { payload })
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

export async function exportBackup(): Promise<string> {
  return invokeI18n<string>('export_backup')
}

export async function importBackup(): Promise<void> {
  return invokeI18n('import_backup')
}
