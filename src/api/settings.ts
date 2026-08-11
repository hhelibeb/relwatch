import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { AppSettings, UpdateSettingsPayload } from '../bindings'

// 类型由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { AppSettings, UpdateSettingsPayload } from '../bindings'

export async function getSettings(): Promise<AppSettings> {
  return invokeI18nFn(commands.getSettings)
}

export async function updateSettings(payload: UpdateSettingsPayload): Promise<void> {
  await invokeI18nFn(() => commands.updateSettings(payload))
}

export async function setDeepseekApiKey(apiKey: string): Promise<void> {
  await invokeI18nFn(() => commands.setDeepseekApiKey(apiKey))
}

export async function setGithubToken(token: string): Promise<void> {
  await invokeI18nFn(() => commands.setGithubToken(token))
}

export async function setYoutubeApiKey(apiKey: string): Promise<void> {
  await invokeI18nFn(() => commands.setYoutubeApiKey(apiKey))
}

export async function setBilibiliCookie(cookie: string): Promise<void> {
  await invokeI18nFn(() => commands.setBilibiliCookie(cookie))
}

/** 判断 base_url 是否为 DeepSeek 官方域名（https + deepseek.com 或其子域）。
 *  保存/测试连接前弹二次确认提示用（审计建议 #1）。 */
export async function isOfficialDeepseekBaseUrl(baseUrl: string): Promise<boolean> {
  return invokeI18nFn(() => commands.isOfficialDeepseekBaseUrl(baseUrl))
}

/** 从登录 WebView 读取 SESSDATA（成功返回 true，未登录抛 err.bili_login_not_logged_in）。 */
export async function readBilibiliLoginCookie(windowLabel: string): Promise<boolean> {
  return invokeI18nFn(() => commands.readBilibiliLoginCookie(windowLabel))
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
