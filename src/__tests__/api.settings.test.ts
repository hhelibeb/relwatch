import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../api/client', () => ({
  invokeI18n: vi.fn(),
  openReleaseUrl: vi.fn(),
  translateError: vi.fn((raw: string) => raw),
}))

import { invokeI18n } from '../api/client'
import {
  getSettings,
  updateSettings,
  setDeepseekApiKey,
  setGithubToken,
  testDeepseekConnection,
  exportBackup,
  importBackup,
} from '../api/settings'
import type { UpdateSettingsPayload } from '../api/settings'

beforeEach(() => {
  vi.clearAllMocks()
})

describe('getSettings', () => {
  it('调起 get_settings 命令，返回完整 AppSettings', async () => {
    const mockSettings = {
      auto_start: false,
      poll_interval_minutes: 30,
      proxy_mode: 'none',
      language: 'zh-CN',
      theme: 'system',
    }
    vi.mocked(invokeI18n).mockResolvedValue(mockSettings)

    const result = await getSettings()

    // 无参数命令：只传 command name
    expect(invokeI18n).toHaveBeenCalledWith('get_settings')
    expect(result.poll_interval_minutes).toBe(30)
    expect(result.language).toBe('zh-CN')
  })
})

describe('updateSettings', () => {
  const payload: UpdateSettingsPayload = {
    pollIntervalMinutes: 15,
    proxyMode: 'none',
    proxyUrl: '',
    autoStart: true,
    minimizeToTray: true,
    logRetentionDays: 30,
    deepseekEnabled: false,
    deepseekModel: 'deepseek-v4-flash',
    deepseekBaseUrl: 'https://api.deepseek.com',
    deepseekProxyBypass: false,
    deepseekPrompt: '',
    deepseekMinImportance: '小',
    deepseekTranslateRelease: false,
    checkPrereleases: false,
    fetchHistory: false,
    fetchHistoryCount: 1,
    language: 'zh-CN',
    theme: 'light',
  }

  it('调起 update_settings 命令并传递 payload', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await updateSettings(payload)

    expect(invokeI18n).toHaveBeenCalledWith('update_settings', { payload })
  })

  it('自定义 payload 正确传递', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await updateSettings({ ...payload, language: 'en-US', theme: 'dark' })

    const callArgs = vi.mocked(invokeI18n).mock.calls[0][1] as { payload: UpdateSettingsPayload }
    expect(callArgs.payload.language).toBe('en-US')
    expect(callArgs.payload.theme).toBe('dark')
  })
})

describe('setDeepseekApiKey', () => {
  it('调起 set_deepseek_api_key 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setDeepseekApiKey('sk-xxxx')

    expect(invokeI18n).toHaveBeenCalledWith('set_deepseek_api_key', { apiKey: 'sk-xxxx' })
  })

  it('空字符串也传递', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setDeepseekApiKey('')

    expect(invokeI18n).toHaveBeenCalledWith('set_deepseek_api_key', { apiKey: '' })
  })
})

describe('setGithubToken', () => {
  it('调起 set_github_token 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setGithubToken('ghp_xxxx')

    expect(invokeI18n).toHaveBeenCalledWith('set_github_token', { token: 'ghp_xxxx' })
  })
})

describe('testDeepseekConnection', () => {
  it('调起 test_deepseek_connection 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue('ok')

    const result = await testDeepseekConnection()

    expect(invokeI18n).toHaveBeenCalledWith('test_deepseek_connection')
    expect(result).toBe('ok')
  })

  it('连接失败时抛出错误', async () => {
    vi.mocked(invokeI18n).mockRejectedValue(new Error('err.deepseek.connection'))

    await expect(testDeepseekConnection()).rejects.toThrow('err.deepseek.connection')
  })
})

describe('exportBackup', () => {
  it('调起 export_backup 命令，返回文件路径', async () => {
    vi.mocked(invokeI18n).mockResolvedValue('/tmp/relwatch-backup.zip')

    const result = await exportBackup()

    expect(invokeI18n).toHaveBeenCalledWith('export_backup')
    expect(result).toBe('/tmp/relwatch-backup.zip')
  })
})

describe('importBackup', () => {
  it('调起 import_backup 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await importBackup()

    expect(invokeI18n).toHaveBeenCalledWith('import_backup')
  })
})
