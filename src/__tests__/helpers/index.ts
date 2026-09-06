/**
 * 共享测试 fixtures
 *
 * 从各测试文件抽取的重复数据工厂，供组件行为测试复用：
 * - createRelease：ReleaseInfo 最小完整对象
 * - defaultSettings：AppSettings 默认值（与 App.vue 初始化一致）
 * - createSource：SourceTab 使用的 Source 字段
 */
import type { ReleaseInfo } from '../../api/releases'
import type { AppSettings } from '../../api/settings'

export function createRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  const now = new Date().toISOString()
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'tauri-apps',
    repo: 'tauri',
    tag_name: 'v2.0.0',
    release_name: 'Tauri 2.0 Stable',
    html_url: 'https://github.com/tauri-apps/tauri/releases/tag/v2.0.0',
    published_at: now,
    prerelease: false,
    body: null,
    detected_at: now,
    notification_status: 'pending' as const,
    snooze_until: null,
    ai_summary: null,
    ai_importance: null,
    body_translated: null,
    extra_metadata: null,
    source_description: null,
    flag: 0,
    version_bump: null,
    ...overrides,
  }
}

export const defaultSettings: AppSettings = {
  auto_start: false,
  poll_interval_minutes: 30,
  proxy_mode: 'none',
  proxy_url: '',
  minimize_to_tray: true,
  log_retention_days: 0,
  deepseek_enabled: false,
  deepseek_model: 'deepseek-v4-flash',
  deepseek_base_url: 'https://api.deepseek.com',
  deepseek_api_key_set: false,
  deepseek_proxy_bypass: false,
  deepseek_prompt: '',
  deepseek_min_importance: '小',
  deepseek_translate_release: false,
  check_prereleases: false,
  fetch_history: false,
  fetch_history_count: 1,
  language: 'zh-CN',
  theme: 'system',
  font_scale: 100,
  show_source_type_icons: true,
  enable_usage_stats: true,
  github_token_set: false,
  youtube_api_key_set: false,
  bilibili_cookie_set: false,
}

export interface TestSource {
  id: number
  source_type: string
  owner: string
  repo: string
  poll_interval_minutes: number
  enabled: boolean
  muted: boolean
  last_checked_at: string | null
  last_check_status: string
  last_check_message: string | null
  consecutive_failures: number
  last_new_count: number
  description: string | null
  created_at: string
  updated_at: string
  config: string | null
}

export function createSource(overrides: Partial<TestSource> = {}): TestSource {
  return {
    id: 1,
    source_type: 'github',
    owner: 'vuejs',
    repo: 'core',
    poll_interval_minutes: 60,
    enabled: true,
    muted: false,
    last_checked_at: '2025-06-01T00:00:00Z',
    last_check_status: 'ok',
    last_check_message: null,
    consecutive_failures: 0,
    last_new_count: 0,
    description: null,
    created_at: '2025-06-01T00:00:00Z',
    updated_at: '2025-06-01T00:00:00Z',
    config: null,
    ...overrides,
  }
}
