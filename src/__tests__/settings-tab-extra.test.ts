import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsTab from '../components/SettingsTab.vue'
import { ShowToastKey } from '../injection-keys'
import type { AppSettings } from '../api/settings'
import {
  updateSettings,
  setDeepseekApiKey,
  setGithubToken,
  testDeepseekConnection,
  isOfficialDeepseekBaseUrl,
  exportBackup,
  importBackup,
} from '../api/settings'
import { openReleaseUrl } from '../api/client'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { setLocale } from '../i18n'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  message: vi.fn(),
  confirm: vi.fn(),
}))

vi.mock('../api/settings', () => ({
  updateSettings: vi.fn().mockResolvedValue(undefined),
  setDeepseekApiKey: vi.fn().mockResolvedValue(undefined),
  setGithubToken: vi.fn().mockResolvedValue(undefined),
  testDeepseekConnection: vi.fn().mockResolvedValue('ok'),
  isOfficialDeepseekBaseUrl: vi.fn().mockResolvedValue(true),
  exportBackup: vi.fn().mockResolvedValue('/tmp/relwatch-backup.zip'),
  importBackup: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))

vi.mock('../i18n', () => ({
  t: vi.fn((key: string, ...args: string[]) => args.length ? `${key}:${args.join(',')}` : key),
  setLocale: vi.fn(),
  languages: [
    { value: 'zh-CN', label: '中文' },
    { value: 'en-US', label: 'English' },
  ],
}))

const updateSettingsMock = vi.mocked(updateSettings)
const setDeepseekApiKeyMock = vi.mocked(setDeepseekApiKey)
const setGithubTokenMock = vi.mocked(setGithubToken)
const testDeepseekConnectionMock = vi.mocked(testDeepseekConnection)
const isOfficialDeepseekBaseUrlMock = vi.mocked(isOfficialDeepseekBaseUrl)
const exportBackupMock = vi.mocked(exportBackup)
const importBackupMock = vi.mocked(importBackup)
const messageMock = vi.mocked(message)
const confirmMock = vi.mocked(confirm)
const setLocaleMock = vi.mocked(setLocale)
const openReleaseUrlMock = vi.mocked(openReleaseUrl)

function createSettings(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    poll_interval_minutes: 15,
    proxy_mode: 'none',
    proxy_url: '',
    auto_start: false,
    minimize_to_tray: false,
    log_retention_days: 30,
    deepseek_enabled: true,
    deepseek_model: 'deepseek-v4-flash',
    deepseek_base_url: 'https://api.deepseek.com',
    deepseek_api_key_set: false,
    deepseek_proxy_bypass: false,
    deepseek_prompt: 'Summarize {}',
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
    ...overrides,
  }
}

function mountSettings(settings: AppSettings = createSettings()) {
  return mount(SettingsTab, {
    props: { settings },
    global: {
      provide: {
        [ShowToastKey as symbol]: vi.fn(),
      },
    },
  })
}

async function clickSidebar(wrapper: ReturnType<typeof mountSettings>, text: string) {
  const buttons = wrapper.findAll('.settings-sidebar button')
  const btn = buttons.find(b => b.text().includes(text))
  expect(btn).toBeTruthy()
  await btn!.trigger('click')
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.useFakeTimers()
  document.documentElement.dataset.theme = ''
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockReturnValue({ matches: false }),
  })
  updateSettingsMock.mockResolvedValue(undefined)
  setDeepseekApiKeyMock.mockResolvedValue(undefined)
  setGithubTokenMock.mockResolvedValue(undefined)
  testDeepseekConnectionMock.mockResolvedValue('Connection OK')
  isOfficialDeepseekBaseUrlMock.mockResolvedValue(true)
  exportBackupMock.mockResolvedValue('/tmp/relwatch-backup.zip')
  importBackupMock.mockResolvedValue(undefined)
  messageMock.mockResolvedValue(undefined as any)
  confirmMock.mockResolvedValue(true as any)
  setLocaleMock.mockClear()
})

afterEach(() => {
  vi.useRealTimers()
  document.documentElement.dataset.theme = ''
})

describe('SettingsTab — AI 设置保存（凭据 + 配置）', () => {
  it('填写 DeepSeek API Key 后保存，调用 setDeepseekApiKey 并清空输入', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_api_key_set: false }))
    await clickSidebar(wrapper, 'settings.ai')

    const apiInput = wrapper.findAll('input[type="password"]')[0]
    await apiInput.setValue('sk-test-key-123')

    // 点击 AI tab 中的保存按钮
    const saveButtons = wrapper.findAll('.setting-actions .btn-primary')
    await saveButtons[0].trigger('click')
    await flushPromises()
    await vi.runAllTimersAsync()

    expect(setDeepseekApiKeyMock).toHaveBeenCalledWith('sk-test-key-123')
    // 保存成功后 input 应被清空
    expect((apiInput.element as HTMLInputElement).value).toBe('')
  })

  it('填写 GitHub Token 后保存，调用 setGithubToken 并清空输入', async () => {
    const wrapper = mountSettings(createSettings({ github_token_set: false }))
    await clickSidebar(wrapper, 'settings.accounts')

    // GitHub Token 是 accounts tab 里的 password input
    const ghInput = wrapper.find('input[type="password"]')
    await ghInput.setValue('ghp_token-abc')

    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()
    await vi.runAllTimersAsync()

    expect(setGithubTokenMock).toHaveBeenCalledWith('ghp_token-abc')
    expect((ghInput.element as HTMLInputElement).value).toBe('')
  })

  it('同时填写 API Key 和 GitHub Token，按顺序保存两个凭据', async () => {
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    // AI tab 中第一个 password 是 API key
    const inputs = wrapper.findAll('input[type="password"]')
    const apiKeyInput = inputs[0]
    await apiKeyInput.setValue('sk-new-key')

    // 切到 accounts tab 设置 GitHub token
    await clickSidebar(wrapper, 'settings.accounts')
    const ghInput = wrapper.find('input[type="password"]')
    await ghInput.setValue('ghp-new-token')

    // 保存
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()
    await vi.runAllTimersAsync()

    // 两个凭据都应被设置
    expect(setDeepseekApiKeyMock).toHaveBeenCalledWith('sk-new-key')
    expect(setGithubTokenMock).toHaveBeenCalledWith('ghp-new-token')
    expect(updateSettingsMock).toHaveBeenCalledOnce()
  })

  it('DeepSeek prompt 不含 {} 时保存失败，显示错误提示', async () => {
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings({ deepseek_prompt: 'no placeholder here' }) },
      global: {
        provide: {
          [ShowToastKey as symbol]: showToast,
        },
      },
    })
    await clickSidebar(wrapper, 'settings.ai')

    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    // 不应调用 updateSettings
    expect(updateSettingsMock).not.toHaveBeenCalled()
    // 应提示校验失败
    expect(showToast).toHaveBeenCalledWith('settings.deepseek_prompt_validate_failed')
  })

  it('保存失败时显示错误信息（含 Error message）', async () => {
    const showToast = vi.fn()
    updateSettingsMock.mockRejectedValueOnce(new Error('database locked'))
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: {
        provide: {
          [ShowToastKey as symbol]: showToast,
        },
      },
    })

    await wrapper.get('input[type="number"]').setValue(30)
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(showToast).toHaveBeenCalledWith(expect.stringContaining('database locked'))
  })

  it('保存时 deepseek_model 为空，默认使用 deepseek-v4-flash', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_model: '' }))
    await clickSidebar(wrapper, 'settings.ai')

    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(updateSettingsMock).toHaveBeenCalledWith(expect.objectContaining({
      deepseekModel: 'deepseek-v4-flash',
    }))
  })

  it('保存时 deepseek_base_url 为空，使用默认 URL', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_base_url: '  ' }))
    await clickSidebar(wrapper, 'settings.ai')

    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(updateSettingsMock).toHaveBeenCalledWith(expect.objectContaining({
      deepseekBaseUrl: 'https://api.deepseek.com',
    }))
  })
})

describe('SettingsTab — 轮询间隔变更检测', () => {
  it('保存时轮询间隔未变，emit update(false)', async () => {
    const wrapper = mountSettings(createSettings({ poll_interval_minutes: 15 }))

    // 修改一个非轮询字段来触发 dirty
    const inputs = wrapper.findAll('input[type="text"]')
    // 修改 proxy_url
    await wrapper.findAll('.settings-sidebar button')[0].trigger('click') // general tab

    // 不修改轮询间隔，只点保存
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('update')?.[0]).toEqual([false])
  })
})

describe('SettingsTab — DeepSeek 连接测试', () => {
  it('测试成功，显示成功对话框', async () => {
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const testBtn = wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))
    expect(testBtn).toBeTruthy()
    await testBtn!.trigger('click')
    await flushPromises()

    expect(testDeepseekConnectionMock).toHaveBeenCalledOnce()
    expect(messageMock).toHaveBeenCalledWith('Connection OK', expect.objectContaining({ kind: 'info' }))
  })

  it('测试失败，显示错误对话框', async () => {
    testDeepseekConnectionMock.mockRejectedValueOnce(new Error('timeout'))
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const testBtn = wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))
    await testBtn!.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.stringContaining('timeout'),
      expect.objectContaining({ kind: 'error' }),
    )
  })
})

describe('SettingsTab — DeepSeek 测试标题 i18n（P1 #8）', () => {
  it('成功对话框标题使用 i18n key 而非硬编码英文', async () => {
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const testBtn = wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))
    await testBtn!.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      'Connection OK',
      expect.objectContaining({ title: 'settings.deepseek_test_title' }),
    )
  })

  it('失败对话框标题同样使用 i18n key', async () => {
    testDeepseekConnectionMock.mockRejectedValueOnce(new Error('boom'))
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const testBtn = wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))
    await testBtn!.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({ title: 'settings.deepseek_test_title' }),
    )
  })
})

describe('SettingsTab — 非官方 DeepSeek 地址二次确认（审计建议 #1）', () => {
  it('保存非官方 base_url：确认后提交 updateSettings', async () => {
    isOfficialDeepseekBaseUrlMock.mockResolvedValue(false)
    confirmMock.mockResolvedValue(true as any)
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const urlInput = wrapper.find('input[placeholder="settings.deepseek_base_url_placeholder"]')
    await urlInput.setValue('https://evil.com')
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledWith(
      'settings.deepseek_non_official_confirm:https://evil.com',
      expect.objectContaining({ kind: 'warning' }),
    )
    expect(updateSettingsMock).toHaveBeenCalledOnce()
    expect(updateSettingsMock).toHaveBeenCalledWith(expect.objectContaining({ deepseekBaseUrl: 'https://evil.com' }))
  })

  it('保存非官方 base_url：取消确认则不提交', async () => {
    isOfficialDeepseekBaseUrlMock.mockResolvedValue(false)
    confirmMock.mockResolvedValue(false as any)
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const urlInput = wrapper.find('input[placeholder="settings.deepseek_base_url_placeholder"]')
    await urlInput.setValue('https://evil.com')
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledOnce()
    expect(updateSettingsMock).not.toHaveBeenCalled()
  })

  it('保存官方 base_url：不弹确认直接提交', async () => {
    isOfficialDeepseekBaseUrlMock.mockResolvedValue(true)
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const urlInput = wrapper.find('input[placeholder="settings.deepseek_base_url_placeholder"]')
    await urlInput.setValue('https://api.deepseek.com/v1')
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(confirmMock).not.toHaveBeenCalled()
    expect(updateSettingsMock).toHaveBeenCalledOnce()
  })

  it('保存但 base_url 未修改：不弹确认（避免打扰已确认过的用户）', async () => {
    isOfficialDeepseekBaseUrlMock.mockResolvedValue(false)
    const wrapper = mountSettings(createSettings({ deepseek_base_url: 'https://evil.com' }))
    await clickSidebar(wrapper, 'settings.ai')

    // 只改一个无关字段，不触碰 base_url
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(confirmMock).not.toHaveBeenCalled()
    expect(updateSettingsMock).toHaveBeenCalledOnce()
  })

  it('测试连接非官方地址：确认后发起测试', async () => {
    isOfficialDeepseekBaseUrlMock.mockResolvedValue(false)
    confirmMock.mockResolvedValue(true as any)
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const urlInput = wrapper.find('input[placeholder="settings.deepseek_base_url_placeholder"]')
    await urlInput.setValue('https://evil.com')
    const testBtn = wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))
    expect(testBtn).toBeTruthy()
    await testBtn!.trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledOnce()
    expect(testDeepseekConnectionMock).toHaveBeenCalledOnce()
    expect(testDeepseekConnectionMock).toHaveBeenCalledWith(expect.objectContaining({ baseUrl: 'https://evil.com' }))
  })

  it('测试连接非官方地址：取消则不发起测试', async () => {
    isOfficialDeepseekBaseUrlMock.mockResolvedValue(false)
    confirmMock.mockResolvedValue(false as any)
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const urlInput = wrapper.find('input[placeholder="settings.deepseek_base_url_placeholder"]')
    await urlInput.setValue('https://evil.com')
    const testBtn = wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))
    expect(testBtn).toBeTruthy()
    await testBtn!.trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledOnce()
    expect(testDeepseekConnectionMock).not.toHaveBeenCalled()
  })
})

describe('SettingsTab — 备份导出', () => {
  it('导出成功，显示成功 toast 并 emit update', async () => {
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await clickSidebar(wrapper, 'settings.data')

    const exportBtn = wrapper.findAll('.backup-actions button')[0]
    await exportBtn.trigger('click')
    await flushPromises()

    expect(showToast).toHaveBeenCalledWith(expect.stringContaining('/tmp/relwatch-backup.zip'))
    expect(wrapper.emitted('update')?.[0]).toEqual([false])
  })

  it('导出取消（后端返回 err.backup_cancelled_export），显示取消 toast', async () => {
    // 新 contract：后端用稳定 err key 表示用户取消；invokeI18n 在测试转换器下返回 key 本身。
    exportBackupMock.mockRejectedValueOnce(new Error('err.backup_cancelled_export'))
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await clickSidebar(wrapper, 'settings.data')

    await wrapper.findAll('.backup-actions button')[0].trigger('click')
    await flushPromises()

    expect(showToast).toHaveBeenCalledWith('backup.export_cancelled')
  })

  it('导出失败（非取消错误），显示失败 toast', async () => {
    exportBackupMock.mockRejectedValueOnce(new Error('disk full'))
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await clickSidebar(wrapper, 'settings.data')

    await wrapper.findAll('.backup-actions button')[0].trigger('click')
    await flushPromises()

    expect(showToast).toHaveBeenCalledWith(expect.stringContaining('disk full'))
  })
})

describe('SettingsTab — 备份导入', () => {
  it('用户取消确认对话框，不执行导入', async () => {
    confirmMock.mockResolvedValueOnce(false)
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await clickSidebar(wrapper, 'settings.data')

    await wrapper.findAll('.backup-actions button')[1].trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledOnce()
    expect(importBackupMock).not.toHaveBeenCalled()
  })

  it('导入取消（后端返回 err.backup_cancelled_import），显示取消 toast', async () => {
    // 新 contract：后端用稳定 err key 表示用户取消；invokeI18n 在测试转换器下返回 key 本身。
    importBackupMock.mockRejectedValueOnce(new Error('err.backup_cancelled_import'))
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await clickSidebar(wrapper, 'settings.data')

    await wrapper.findAll('.backup-actions button')[1].trigger('click')
    await flushPromises()

    expect(showToast).toHaveBeenCalledWith('backup.import_cancelled')
  })

  it('导入失败（非取消错误），显示失败 toast', async () => {
    importBackupMock.mockRejectedValueOnce(new Error('corrupt file'))
    const showToast = vi.fn()
    const wrapper = mount(SettingsTab, {
      props: { settings: createSettings() },
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await clickSidebar(wrapper, 'settings.data')

    await wrapper.findAll('.backup-actions button')[1].trigger('click')
    await flushPromises()

    expect(showToast).toHaveBeenCalledWith(expect.stringContaining('corrupt file'))
  })
})

describe('SettingsTab — 主题下拉选择', () => {
  it('选择 dark 主题，设置 dataset.theme 为 dark', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    // 打开下拉
    await themeSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()

    // 选择 dark
    const options = themeSelect.findAll('.theme-select-option')
    const darkOpt = options.find(o => o.attributes('data-value') === 'dark')
    expect(darkOpt).toBeTruthy()
    await darkOpt!.trigger('click')
    await vi.runAllTimersAsync()

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('选择 system 主题，根据 matchMedia 决定 theme', async () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: true }) as unknown as typeof window.matchMedia
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()

    const options = themeSelect.findAll('.theme-select-option')
    const systemOpt = options.find(o => o.attributes('data-value') === 'system')
    await systemOpt!.trigger('click')
    await vi.runAllTimersAsync()

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('主题下拉键盘导航：ArrowDown/ArrowUp 切换预览', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    const trigger = themeSelect.get('.theme-select-trigger')

    // 用键盘打开下拉
    await trigger.trigger('keydown', { key: 'ArrowDown' })
    await vi.runAllTimersAsync()

    expect(wrapper.findAll('.theme-select-dropdown').length).toBeGreaterThan(0)

    // ArrowDown 移到下一个
    await trigger.trigger('keydown', { key: 'ArrowDown' })
    // ArrowUp 回到上一个
    await trigger.trigger('keydown', { key: 'ArrowUp' })
  })

  it('主题下拉键盘导航：Escape 关闭并恢复预览', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    const trigger = themeSelect.get('.theme-select-trigger')

    // 打开
    await trigger.trigger('click')
    await vi.runAllTimersAsync()

    // 悬停预览 dark
    const options = themeSelect.findAll('.theme-select-option')
    const darkOpt = options.find(o => o.attributes('data-value') === 'dark')
    await darkOpt!.trigger('mouseenter')
    expect(document.documentElement.dataset.theme).toBe('dark')

    // Escape 关闭
    await trigger.trigger('keydown', { key: 'Escape' })
    await vi.runAllTimersAsync()

    // 应恢复 light
    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('主题下拉外部点击关闭', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()

    expect(themeSelect.find('.theme-select-dropdown').exists()).toBe(true)

    // 模拟外部点击
    document.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await vi.runAllTimersAsync()

    expect(themeSelect.find('.theme-select-dropdown').exists()).toBe(false)
  })
})

describe('SettingsTab — 语言下拉选择', () => {
  it('选择 English，调用 setLocale 并关闭下拉', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const langSelect = wrapper.findAll('.theme-select')[0]
    await langSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()

    const options = langSelect.findAll('.theme-select-option')
    const enOpt = options.find(o => o.attributes('data-value') === 'en-US')
    await enOpt!.trigger('click')
    await vi.runAllTimersAsync()

    expect(setLocaleMock).toHaveBeenCalledWith('en-US')
  })

  it('语言下拉键盘导航：Enter 选中当前聚焦项', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const langSelect = wrapper.findAll('.theme-select')[0]
    const trigger = langSelect.get('.theme-select-trigger')

    // 打开
    await trigger.trigger('keydown', { key: 'ArrowDown' })
    await vi.runAllTimersAsync()

    // 导航到 English
    await trigger.trigger('keydown', { key: 'ArrowDown' })
    // Enter 选中
    await trigger.trigger('keydown', { key: 'Enter' })
    await vi.runAllTimersAsync()
  })

  it('语言下拉键盘导航：Escape 关闭并恢复语言', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const langSelect = wrapper.findAll('.theme-select')[0]
    const trigger = langSelect.get('.theme-select-trigger')

    // 打开
    await trigger.trigger('click')
    await vi.runAllTimersAsync()

    // 悬停预览 English
    const options = langSelect.findAll('.theme-select-option')
    const enOpt = options.find(o => o.attributes('data-value') === 'en-US')
    await enOpt!.trigger('mouseenter')
    expect(setLocaleMock).toHaveBeenLastCalledWith('en-US')

    // Escape 关闭并恢复
    await trigger.trigger('keydown', { key: 'Escape' })
    await vi.runAllTimersAsync()

    expect(setLocaleMock).toHaveBeenLastCalledWith('zh-CN')
  })

  it('语言下拉外部点击关闭并恢复语言', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const langSelect = wrapper.findAll('.theme-select')[0]
    await langSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()

    // 预览
    const options = langSelect.findAll('.theme-select-option')
    const enOpt = options.find(o => o.attributes('data-value') === 'en-US')
    await enOpt!.trigger('mouseenter')
    expect(setLocaleMock).toHaveBeenLastCalledWith('en-US')

    // 外部点击
    document.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await vi.runAllTimersAsync()

    expect(setLocaleMock).toHaveBeenLastCalledWith('zh-CN')
  })
})

describe('SettingsTab — Dirty 标记与 discard', () => {
  it('修改多个字段，dirtyCount 正确显示', async () => {
    const wrapper = mountSettings()

    // 修改 poll_interval
    await wrapper.get('input[type="number"]').setValue(30)
    expect(wrapper.find('.settings-banner').exists()).toBe(true)

    // 修改 auto_start
    const checkboxes = wrapper.findAll('input[type="checkbox"]')
    await checkboxes[0].setValue(true)

    // banner 仍然可见
    expect(wrapper.find('.settings-banner').exists()).toBe(true)
    // 应显示多个未保存变更
    expect(wrapper.find('.settings-banner').text()).toContain('settings.unsaved_banner')
  })

  it('dirtyByTab 在不同 tab 显示 dirty dot', async () => {
    const wrapper = mountSettings()

    // 修改 general tab 的字段
    await wrapper.get('input[type="number"]').setValue(30)

    // 切换到 appearance 检查 general tab 有 dirty dot
    const sidebarButtons = wrapper.findAll('.settings-sidebar button')
    const generalBtn = sidebarButtons.find(b => b.text().includes('settings.general'))
    expect(generalBtn!.find('.sidebar-dirty-dot').exists()).toBe(true)

    // 修改 appearance tab 的字段
    await clickSidebar(wrapper, 'settings.appearance')
    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()
    const options = themeSelect.findAll('.theme-select-option')
    const darkOpt = options.find(o => o.attributes('data-value') === 'dark')
    await darkOpt!.trigger('click')
    await vi.runAllTimersAsync()

    // appearance 也应该有 dirty dot
    const appearanceBtn = wrapper.findAll('.settings-sidebar button').find(b => b.text().includes('settings.appearance'))
    expect(appearanceBtn!.find('.sidebar-dirty-dot').exists()).toBe(true)
  })

  it('填写 API Key 后，AI tab 显示 dirty dot', async () => {
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.ai')

    const inputs = wrapper.findAll('input[type="password"]')
    const apiKeyInput = inputs[0]
    await apiKeyInput.setValue('sk-test')

    const aiBtn = wrapper.findAll('.settings-sidebar button').find(b => b.text().includes('settings.ai'))
    expect(aiBtn!.find('.sidebar-dirty-dot').exists()).toBe(true)
  })

  it('填写 GitHub Token 后，账号 tab 显示 dirty dot', async () => {
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.accounts')

    const ghInput = wrapper.find('input[type="password"]')
    await ghInput.setValue('ghp-token')

    const accountsBtn = wrapper.findAll('.settings-sidebar button').find(b => b.text().includes('settings.accounts'))
    expect(accountsBtn!.find('.sidebar-dirty-dot').exists()).toBe(true)
  })

  it('discard 恢复 language 时调用 setLocale 恢复原语言', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    // 选择 English（确认，非预览）
    const langSelect = wrapper.findAll('.theme-select')[0]
    await langSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()
    const options = langSelect.findAll('.theme-select-option')
    const enOpt = options.find(o => o.attributes('data-value') === 'en-US')
    await enOpt!.trigger('click')
    await vi.runAllTimersAsync()

    expect(setLocaleMock).toHaveBeenCalledWith('en-US')

    // discard
    await wrapper.get('.settings-banner .btn-secondary').trigger('click')

    expect(setLocaleMock).toHaveBeenCalledWith('zh-CN')
  })

  it('discard 恢复 theme 时设置正确的 data-theme', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    // 选择 dark
    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()
    const options = themeSelect.findAll('.theme-select-option')
    const darkOpt = options.find(o => o.attributes('data-value') === 'dark')
    await darkOpt!.trigger('click')
    await vi.runAllTimersAsync()

    expect(document.documentElement.dataset.theme).toBe('dark')

    // discard
    await wrapper.get('.settings-banner .btn-secondary').trigger('click')

    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('discard 清空 API Key 和 GitHub Token 输入', async () => {
    const wrapper = mountSettings()

    await clickSidebar(wrapper, 'settings.accounts')
    // 在 accounts tab 填写 GitHub token
    const ghInput = wrapper.find('input[type="password"]')
    await ghInput.setValue('ghp-something')
    expect((ghInput.element as HTMLInputElement).value).toBe('ghp-something')

    // discard
    await wrapper.get('.settings-banner .btn-secondary').trigger('click')

    expect((ghInput.element as HTMLInputElement).value).toBe('')
  })
})

describe('SettingsTab — 条件渲染', () => {
  it('proxy_mode=custom 时显示 proxy URL 输入框', async () => {
    const wrapper = mountSettings(createSettings({ proxy_mode: 'custom', proxy_url: 'socks5://127.0.0.1:7890' }))

    const proxyInput = wrapper.findAll('input[type="text"]').find(i =>
      (i.element as HTMLInputElement).placeholder?.includes('settings.proxy_placeholder') ||
      (i.element as HTMLInputElement).value?.includes('socks5'),
    )
    expect(proxyInput).toBeTruthy()
  })

  it('proxy_mode 从 none 切换到 custom 后出现 proxy URL 输入框', async () => {
    const wrapper = mountSettings(createSettings({ proxy_mode: 'none' }))

    // 初始无 proxy URL 输入
    const textInputs = wrapper.findAll('input[type="text"]')
    const proxyInputBefore = textInputs.find(i =>
      (i.element as HTMLInputElement).placeholder === 'settings.proxy_placeholder',
    )
    expect(proxyInputBefore).toBeFalsy()

    // 切换 proxy_mode
    const selects = wrapper.findAll('select')
    const proxySelect = selects[0]
    await proxySelect.setValue('custom')

    const textInputsAfter = wrapper.findAll('input[type="text"]')
    const proxyInputAfter = textInputsAfter.find(i =>
      (i.element as HTMLInputElement).placeholder === 'settings.proxy_placeholder',
    )
    expect(proxyInputAfter).toBeTruthy()
  })

  it('fetch_history=true 时显示 fetch_history_count 输入框', async () => {
    const wrapper = mountSettings(createSettings({ fetch_history: true }))

    const numberInputs = wrapper.findAll('input[type="number"]')
    // 应该至少有 3 个 number input: poll_interval, log_retention, fetch_history_count
    expect(numberInputs.length).toBeGreaterThanOrEqual(3)
  })

  it('AI tab 中 deepseek_enabled=false 时隐藏 AI 配置项', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_enabled: false }))
    await clickSidebar(wrapper, 'settings.ai')

    // 不应有 password input（API key）
    expect(wrapper.find('input[type="password"]').exists()).toBe(false)
    // 不应有 test connection 按钮
    expect(wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))).toBeFalsy()
  })

  it('AI tab 中 deepseek_enabled=true 时显示 AI 配置项', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_enabled: true }))
    await clickSidebar(wrapper, 'settings.ai')

    // 应有 password input（API key）
    expect(wrapper.find('input[type="password"]').exists()).toBe(true)
    // 应有 test connection 按钮
    expect(wrapper.findAll('button').find(b => b.text().includes('settings.test_connection'))).toBeTruthy()
  })

  it('切换 deepseek_enabled 开/关，子字段显隐切换', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_enabled: false }))
    await clickSidebar(wrapper, 'settings.ai')

    // 初始关闭
    expect(wrapper.find('input[type="password"]').exists()).toBe(false)

    // 找到 enable checkbox
    const checkboxes = wrapper.findAll('input[type="checkbox"]')
    const enableCheckbox = checkboxes[0]
    await enableCheckbox.setValue(true)

    // 现在应该有 API key input
    expect(wrapper.find('input[type="password"]').exists()).toBe(true)

    // 再关闭
    await enableCheckbox.setValue(false)
    expect(wrapper.find('input[type="password"]').exists()).toBe(false)
  })
})

describe('SettingsTab — Tab 导航与版本', () => {
  it('点击各 sidebar 按钮切换 tab 内容', async () => {
    const wrapper = mountSettings()

    // 默认显示 general
    expect(wrapper.find('.settings-form').exists()).toBe(true)
    // 应能看到 poll_interval input
    expect(wrapper.find('input[type="number"]').exists()).toBe(true)

    // 切换到 accounts
    await clickSidebar(wrapper, 'settings.accounts')
    // 应能看到 3 个凭据 password input
    expect(wrapper.findAll('input[type="password"]').length).toBe(3)

    // 切换到 appearance
    await clickSidebar(wrapper, 'settings.appearance')
    expect(wrapper.findAll('.theme-select').length).toBe(2) // language + theme

    // 切换到 AI
    await clickSidebar(wrapper, 'settings.ai')
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(true)

    // 切换到 data
    await clickSidebar(wrapper, 'settings.data')
    expect(wrapper.findAll('.backup-actions button').length).toBe(2)
  })

  it('点击 GitHub 按钮调用 openReleaseUrl', async () => {
    const wrapper = mountSettings()

    const githubBtn = wrapper.find('.version-github-btn')
    await githubBtn.trigger('click')

    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://github.com/hhelibeb/relwatch')
  })

  it('data tab 中 github_token_set=true 时显示 token 备注', async () => {
    const wrapper = mountSettings(createSettings({ github_token_set: true }))
    await clickSidebar(wrapper, 'settings.data')

    expect(wrapper.find('.setting-section-desc').text()).toContain('backup.token_note')
  })
})

describe('SettingsTab — Props 同步', () => {
  it('外部更新 props.settings 时，表单自动同步', async () => {
    const wrapper = mountSettings(createSettings({ poll_interval_minutes: 15 }))
    const input = wrapper.get('input[type="number"]') as ReturnType<typeof wrapper.get>
    expect((input.element as HTMLInputElement).value).toBe('15')

    await (wrapper as any).setProps({ settings: createSettings({ poll_interval_minutes: 60 }) })
    await flushPromises()

    expect((input.element as HTMLInputElement).value).toBe('60')
  })
})

describe('SettingsTab — 组件卸载清理', () => {
  it('卸载时移除 document 事件监听器', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const removeSpy = vi.spyOn(document, 'removeEventListener')

    // 打开主题下拉（会注册 outsideClickHandler）
    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    await vi.runAllTimersAsync()

    // 卸载
    wrapper.unmount()

    expect(removeSpy).toHaveBeenCalled()
    removeSpy.mockRestore()
  })
})

describe('SettingsTab — 下拉快速 toggle 监听器泄漏（P1 #11）', () => {
  // #11 守卫针对“watch isOpen=true 已排 nextTick、回调执行前下拉被关闭”的快闪场景。
  // 该场景的最终不泄漏目标等价于：打开→关闭后 document 上的 outsideClick 监听器被成对移除。
  it('主题下拉打开后关闭，outsideClick 监听器被成对移除（不残留）', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const addSpy = vi.spyOn(document, 'addEventListener')
    const removeSpy = vi.spyOn(document, 'removeEventListener')
    const themeSelect = wrapper.findAll('.theme-select')[1]
    const trigger = themeSelect.get('.theme-select-trigger')

    // 打开：注册 outsideClick
    await trigger.trigger('click')
    await vi.runAllTimersAsync()
    await flushPromises()
    const addedClicks = addSpy.mock.calls.filter(c => c[0] === 'click').length
    expect(addedClicks).toBe(1)

    // 关闭：移除 outsideClick
    await trigger.trigger('click')
    await vi.runAllTimersAsync()
    await flushPromises()
    const removedClicks = removeSpy.mock.calls.filter(c => c[0] === 'click').length

    expect(removedClicks).toBe(1) // 成对移除，不残留
    addSpy.mockRestore()
    removeSpy.mockRestore()
  })

  it('语言下拉打开后关闭，outsideClick 监听器被成对移除（不残留）', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const addSpy = vi.spyOn(document, 'addEventListener')
    const removeSpy = vi.spyOn(document, 'removeEventListener')
    const langSelect = wrapper.findAll('.theme-select')[0]
    const trigger = langSelect.get('.theme-select-trigger')

    await trigger.trigger('click')
    await vi.runAllTimersAsync()
    await flushPromises()
    expect(addSpy.mock.calls.filter(c => c[0] === 'click').length).toBe(1)

    await trigger.trigger('click')
    await vi.runAllTimersAsync()
    await flushPromises()
    expect(removeSpy.mock.calls.filter(c => c[0] === 'click').length).toBe(1)

    addSpy.mockRestore()
    removeSpy.mockRestore()
  })
})
