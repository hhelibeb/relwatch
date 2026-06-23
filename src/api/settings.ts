import { invokeI18n } from './client'

export interface AppSettings {
  poll_interval_minutes: number
  proxy_mode: string
  proxy_url: string
  auto_start: boolean
  minimize_to_tray: boolean
  log_retention_days: number
  deepseek_enabled: boolean
  deepseek_model: string
  deepseek_base_url: string
  deepseek_api_key_set: boolean
  deepseek_proxy_bypass: boolean
  deepseek_prompt: string
  deepseek_min_importance: string
  deepseek_translate_release: boolean

  check_prereleases: boolean
  fetch_history: boolean
  fetch_history_count: number
  language: string
  theme: string
  show_source_type_icons: boolean
  github_token_set: boolean
}

export interface UpdateSettingsPayload {
  pollIntervalMinutes: number
  proxyMode: string
  proxyUrl: string
  autoStart: boolean
  minimizeToTray: boolean
  logRetentionDays: number
  deepseekEnabled: boolean
  deepseekModel: string
  deepseekBaseUrl: string
  deepseekProxyBypass: boolean
  deepseekPrompt: string
  deepseekMinImportance: string
  deepseekTranslateRelease: boolean

  checkPrereleases: boolean
  fetchHistory: boolean
  fetchHistoryCount: number
  language: string
  theme: string
  showSourceTypeIcons: boolean
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
