import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { AppSettings } from '../bindings'

// 类型由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { AppSettings } from '../bindings'

/** 凭据 kind：与后端 `CREDENTIAL_KINDS` 注册表一一对应（M2）。 */
export type CredentialKind = 'deepseek_api_key' | 'github_token' | 'youtube_api_key' | 'bilibili_cookie'

export async function getSettings(): Promise<AppSettings> {
  return invokeI18nFn(commands.getSettings)
}

export async function updateSettings(payload: AppSettings): Promise<void> {
  await invokeI18nFn(() => commands.updateSettings(payload))
}

/** 设置/更新单个加密凭据：空值清除，非空值加密存储（后端 set_credential，M2）。 */
export async function setCredential(kind: CredentialKind, value: string): Promise<void> {
  await invokeI18nFn(() => commands.setCredential(kind, value))
}

/** 首屏加载完成前的类型占位默认值（与后端 get_settings 默认语义一致）。
 *  真实值由 `getSettings()` 首屏整体覆盖；新增设置项时 TS 类型强制此处补齐，
 *  无需手工维护第二份默认值清单。 */
export const DEFAULT_SETTINGS: AppSettings = {
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
  show_importance: false,
  enable_usage_stats: true,
  github_token_set: false,
  youtube_api_key_set: false,
  bilibili_cookie_set: false,
}

/** 从登录 WebView 读取 SESSDATA（成功返回 true，未登录抛 err.bili_login_not_logged_in）。 */
export async function readBilibiliLoginCookie(windowLabel: string): Promise<boolean> {
  return invokeI18nFn(() => commands.readBilibiliLoginCookie(windowLabel))
}

/** 由 Rust 建窗打开 B 站登录窗口（可注入应用代理）。窗口已存在时后端幂等返回。 */
export async function openBilibiliLoginWindow(title: string): Promise<void> {
  await invokeI18nFn(() => commands.openBilibiliLoginWindow(title))
}

export async function closeBilibiliLoginWindow(windowLabel: string): Promise<void> {
  await invokeI18nFn(() => commands.closeBilibiliLoginWindow(windowLabel))
}

/**
 * 测试连接的可选覆盖参数：传表单当前值（含未保存修改），
 * 留空的字段由后端回退到已保存配置，实现"先试后存"。
 * （bindings 侧为全必填可空类型，包装时转换 undefined → null）
 */
export interface TestDeepseekPayload {
  model?: string
  baseUrl?: string
  apiKey?: string
  proxyBypass?: boolean
  proxyUrl?: string
  proxyMode?: string
}

export async function testDeepseekConnection(payload?: TestDeepseekPayload): Promise<void> {
  await invokeI18nFn(() => commands.testDeepseekConnection(payload
    ? {
        model: payload.model ?? null,
        baseUrl: payload.baseUrl ?? null,
        apiKey: payload.apiKey ?? null,
        proxyBypass: payload.proxyBypass ?? null,
        proxyUrl: payload.proxyUrl ?? null,
        proxyMode: payload.proxyMode ?? null,
      }
    : null))
}

export async function exportBackup(): Promise<string> {
  return invokeI18nFn(commands.exportBackup)
}

export async function importBackup(): Promise<void> {
  await invokeI18nFn(commands.importBackup)
}
