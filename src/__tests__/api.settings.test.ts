import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('../api/client', () => ({
  // 直接执行传入的绑定命令函数，使断言落在 invoke 层（命令名+参数与 Rust 端一致）
  invokeI18nFn: vi.fn(async <T>(fn: () => Promise<T>) => fn()),
  openReleaseUrl: vi.fn(),
  translateError: vi.fn((raw: string) => raw),
}))

import { invoke } from '@tauri-apps/api/core'
import {
  getSettings,
  updateSettings,
  setCredential,
  isOfficialDeepseekBaseUrl,
  testDeepseekConnection,
  exportBackup,
  importBackup,
} from '../api/settings'
import type { AppSettings } from '../api/settings'

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
    vi.mocked(invoke).mockResolvedValue(mockSettings)

    const result = await getSettings()

    // 无参数命令：只传 command name
    expect(invoke).toHaveBeenCalledWith('get_settings')
    expect(result.poll_interval_minutes).toBe(30)
    expect(result.language).toBe('zh-CN')
  })
})

describe('updateSettings', () => {
  const payload: AppSettings = {
    poll_interval_minutes: 15,
    proxy_mode: 'none',
    proxy_url: '',
    auto_start: true,
    minimize_to_tray: true,
    log_retention_days: 30,
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
    theme: 'light',
    show_source_type_icons: true,
    enable_usage_stats: true,
    github_token_set: false,
    youtube_api_key_set: false,
    bilibili_cookie_set: false,
  }

  it('调起 update_settings 命令并传递 payload', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await updateSettings(payload)

    expect(invoke).toHaveBeenCalledWith('update_settings', { payload })
  })

  it('自定义 payload 正确传递', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await updateSettings({ ...payload, language: 'en-US', theme: 'dark' })

    const callArgs = vi.mocked(invoke).mock.calls[0][1] as { payload: AppSettings }
    expect(callArgs.payload.language).toBe('en-US')
    expect(callArgs.payload.theme).toBe('dark')
  })
})

describe('setCredential', () => {
  it('deepseek_api_key 调起 set_credential 命令并传递 kind/value', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await setCredential('deepseek_api_key', 'sk-xxxx')

    expect(invoke).toHaveBeenCalledWith('set_credential', { kind: 'deepseek_api_key', value: 'sk-xxxx' })
  })

  it('github_token 调起 set_credential 命令', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await setCredential('github_token', 'ghp_xxxx')

    expect(invoke).toHaveBeenCalledWith('set_credential', { kind: 'github_token', value: 'ghp_xxxx' })
  })

  it('youtube_api_key 调起 set_credential 命令', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await setCredential('youtube_api_key', 'AIzaSy_xxx')

    expect(invoke).toHaveBeenCalledWith('set_credential', { kind: 'youtube_api_key', value: 'AIzaSy_xxx' })
  })

  it('bilibili_cookie 空值清除也传递', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await setCredential('bilibili_cookie', '')

    expect(invoke).toHaveBeenCalledWith('set_credential', { kind: 'bilibili_cookie', value: '' })
  })
})

describe('isOfficialDeepseekBaseUrl', () => {
  it('调起 is_official_deepseek_base_url 命令并透传返回值', async () => {
    vi.mocked(invoke).mockResolvedValue(false)

    const result = await isOfficialDeepseekBaseUrl('https://evil.com')

    expect(invoke).toHaveBeenCalledWith('is_official_deepseek_base_url', { baseUrl: 'https://evil.com' })
    expect(result).toBe(false)
  })
})

describe('testDeepseekConnection', () => {
  it('无 payload 时传 null', async () => {
    vi.mocked(invoke).mockResolvedValue(null)

    const result = await testDeepseekConnection()

    expect(invoke).toHaveBeenCalledWith('test_deepseek_connection', { payload: null })
    expect(result).toBeUndefined()
  })

  it('连接失败时抛出错误', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('err.deepseek.connection'))

    await expect(testDeepseekConnection()).rejects.toThrow('err.deepseek.connection')
  })

  it('携带表单覆盖参数时透传 payload（未填字段转 null）', async () => {
    vi.mocked(invoke).mockResolvedValue(null)

    await testDeepseekConnection({
      model: 'deepseek-v4',
      baseUrl: 'https://api.example.com',
      apiKey: 'sk-new',
      proxyBypass: true,
      proxyUrl: 'http://127.0.0.1:7890',
      proxyMode: 'custom',
    })

    expect(invoke).toHaveBeenCalledWith('test_deepseek_connection', {
      payload: {
        model: 'deepseek-v4',
        baseUrl: 'https://api.example.com',
        apiKey: 'sk-new',
        proxyBypass: true,
        proxyUrl: 'http://127.0.0.1:7890',
        proxyMode: 'custom',
      },
    })
  })

  it('部分字段缺省时转换为 null', async () => {
    vi.mocked(invoke).mockResolvedValue(null)

    await testDeepseekConnection({ model: 'deepseek-v4' })

    expect(invoke).toHaveBeenCalledWith('test_deepseek_connection', {
      payload: {
        model: 'deepseek-v4',
        baseUrl: null,
        apiKey: null,
        proxyBypass: null,
        proxyUrl: null,
        proxyMode: null,
      },
    })
  })
})

describe('exportBackup', () => {
  it('调起 export_backup 命令，返回文件路径', async () => {
    vi.mocked(invoke).mockResolvedValue('/tmp/relwatch-backup.zip')

    const result = await exportBackup()

    expect(invoke).toHaveBeenCalledWith('export_backup')
    expect(result).toBe('/tmp/relwatch-backup.zip')
  })
})

describe('importBackup', () => {
  it('调起 import_backup 命令', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await importBackup()

    expect(invoke).toHaveBeenCalledWith('import_backup')
  })
})
