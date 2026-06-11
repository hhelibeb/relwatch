import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsTab from '../components/SettingsTab.vue'
import { ShowToastKey } from '../injection-keys'
import type { AppSettings } from '../api/settings'
import {
  updateSettings,
  setDeepseekApiKey,
  setGithubToken,
  importBackup,
} from '../api/settings'
import { confirm } from '@tauri-apps/plugin-dialog'
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
const importBackupMock = vi.mocked(importBackup)
const confirmMock = vi.mocked(confirm)
const setLocaleMock = vi.mocked(setLocale)

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
    check_prereleases: false,
    fetch_history: false,
    fetch_history_count: 1,
    language: 'zh-CN',
    theme: 'light',
    github_token_set: false,
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

async function clickSidebar(wrapper: ReturnType<typeof mountSettings>, key: string) {
  const button = wrapper.findAll('.settings-sidebar button').find(btn => btn.text().includes(key))
  expect(button, `sidebar button ${key} should exist`).toBeTruthy()
  await button!.trigger('click')
}

beforeEach(() => {
  vi.clearAllMocks()
  document.documentElement.dataset.theme = ''
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockReturnValue({ matches: false }),
  })
  updateSettingsMock.mockResolvedValue(undefined)
  setDeepseekApiKeyMock.mockResolvedValue(undefined)
  setGithubTokenMock.mockResolvedValue(undefined)
  importBackupMock.mockResolvedValue(undefined)
  confirmMock.mockResolvedValue(true)
})

afterEach(() => {
  document.documentElement.dataset.theme = ''
})

describe('SettingsTab — 表单保护性行为', () => {
  it('修改字段后出现未保存 banner', async () => {
    const wrapper = mountSettings()

    expect(wrapper.find('.settings-banner').exists()).toBe(false)
    await wrapper.get('input[type="number"]').setValue(30)

    expect(wrapper.find('.settings-banner').exists()).toBe(true)
    expect(wrapper.find('.settings-banner').text()).toContain('settings.unsaved_banner')
  })

  it('discard 后恢复 props 值并隐藏未保存 banner', async () => {
    const wrapper = mountSettings(createSettings({ poll_interval_minutes: 15 }))
    const pollInput = wrapper.get('input[type="number"]')

    await pollInput.setValue(45)
    expect((pollInput.element as HTMLInputElement).value).toBe('45')
    expect(wrapper.find('.settings-banner').exists()).toBe(true)

    await wrapper.get('.settings-banner .btn-secondary').trigger('click')

    expect((pollInput.element as HTMLInputElement).value).toBe('15')
    expect(wrapper.find('.settings-banner').exists()).toBe(false)
  })

  it('save 时调用 updateSettings，并把表单字段映射为后端 payload', async () => {
    const wrapper = mountSettings()

    await wrapper.get('input[type="number"]').setValue(25)
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(updateSettingsMock).toHaveBeenCalledOnce()
    expect(updateSettingsMock).toHaveBeenCalledWith(expect.objectContaining({
      pollIntervalMinutes: 25,
      proxyMode: 'none',
      deepseekEnabled: true,
      deepseekModel: 'deepseek-v4-flash',
      language: 'zh-CN',
      theme: 'light',
    }))
    expect(wrapper.emitted('update')?.[0]).toEqual([true])
  })

  it('语言预览后离开恢复为当前表单语言', async () => {
    const wrapper = mountSettings(createSettings({ language: 'zh-CN' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const languageSelect = wrapper.findAll('.theme-select')[0]
    await languageSelect.get('.theme-select-trigger').trigger('click')
    const englishOption = languageSelect.findAll('.theme-select-option').find(option => option.text() === 'English')
    expect(englishOption).toBeTruthy()

    await englishOption!.trigger('mouseenter')
    expect(setLocaleMock).toHaveBeenLastCalledWith('en-US')

    await languageSelect.trigger('mouseleave')
    expect(setLocaleMock).toHaveBeenLastCalledWith('zh-CN')
  })

  it('主题预览后离开恢复为当前表单主题', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    const darkOption = themeSelect.findAll('.theme-select-option').find(option => option.text() === 'settings.theme_dark')
    expect(darkOption).toBeTruthy()

    await darkOption!.trigger('mouseenter')
    expect(document.documentElement.dataset.theme).toBe('dark')

    await themeSelect.trigger('mouseleave')
    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('AI key / GitHub token 留空时不调用对应设置接口', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_api_key_set: true, github_token_set: true }))

    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(updateSettingsMock).toHaveBeenCalledOnce()
    expect(setDeepseekApiKeyMock).not.toHaveBeenCalled()
    expect(setGithubTokenMock).not.toHaveBeenCalled()
  })

  it('导入备份成功后 emit forceReload', async () => {
    const wrapper = mountSettings()
    await clickSidebar(wrapper, 'settings.data')

    const importButton = wrapper.findAll('.backup-actions button')[1]
    await importButton.trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledOnce()
    expect(importBackupMock).toHaveBeenCalledOnce()
    expect(wrapper.emitted('update')?.[0]).toEqual([false, true])
  })
})
