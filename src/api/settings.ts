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
  youtube_api_key_set: boolean
  bilibili_cookie_set: boolean
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

export async function setYoutubeApiKey(apiKey: string): Promise<void> {
  return invokeI18n('set_youtube_api_key', { apiKey })
}

export async function setBilibiliCookie(cookie: string): Promise<void> {
  return invokeI18n('set_bilibili_cookie', { cookie })
}

/** 判断 base_url 是否为 DeepSeek 官方域名（https + deepseek.com 或其子域）。
 *  保存/测试连接前弹二次确认提示用（审计建议 #1）。 */
export async function isOfficialDeepseekBaseUrl(baseUrl: string): Promise<boolean> {
  return invokeI18n<boolean>('is_official_deepseek_base_url', { baseUrl })
}

/** 从登录 WebView 读取 SESSDATA（成功返回 true，未登录抛 err.bili_login_not_logged_in）。 */
export async function readBilibiliLoginCookie(windowLabel: string): Promise<boolean> {
  return invokeI18n<boolean>('read_bilibili_login_cookie', { windowLabel })
}

export async function closeBilibiliLoginWindow(windowLabel: string): Promise<void> {
  return invokeI18n('close_bilibili_login_window', { windowLabel })
}

/**
 * 测试连接的可选覆盖参数：传表单当前值（含未保存修改），
 * 留空的字段由后端回退到已保存配置，实现"先试后存"。
 */
export interface TestDeepseekPayload {
  model?: string
  baseUrl?: string
  apiKey?: string
  proxyBypass?: boolean
  proxyUrl?: string
  proxyMode?: string
}

export async function testDeepseekConnection(payload?: TestDeepseekPayload): Promise<string> {
  return invokeI18n<string>('test_deepseek_connection', payload ? { payload } : undefined)
}

export async function exportBackup(): Promise<string> {
  return invokeI18n<string>('export_backup')
}

export async function importBackup(): Promise<void> {
  return invokeI18n('import_backup')
}
