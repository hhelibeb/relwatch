import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsTab from '../components/SettingsTab.vue'
import { ShowToastKey } from '../injection-keys'
import type { AppSettings } from '../api/settings'
import {
  updateSettings,
  setCredential,
  importBackup,
} from '../api/settings'
import { confirm } from '@tauri-apps/plugin-dialog'
import { setLocale, getLocale, t } from '../i18n'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  message: vi.fn(),
  confirm: vi.fn(),
}))

vi.mock('../api/settings', () => ({
  updateSettings: vi.fn().mockResolvedValue(undefined),
  setCredential: vi.fn().mockResolvedValue(undefined),
  testDeepseekConnection: vi.fn().mockResolvedValue('ok'),
  exportBackup: vi.fn().mockResolvedValue('/tmp/relwatch-backup.zip'),
  importBackup: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))

// i18n 为纯内存模块：不 mock，直接用真实字典
const updateSettingsMock = vi.mocked(updateSettings)
const setCredentialMock = vi.mocked(setCredential)
const importBackupMock = vi.mocked(importBackup)
const confirmMock = vi.mocked(confirm)

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
    font_scale: 100,
    show_source_type_icons: true,
    show_importance: false,
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

async function clickSidebar(wrapper: ReturnType<typeof mountSettings>, key: string) {
  const button = wrapper.findAll('.settings-sidebar button').find(btn => btn.text().includes(t(key)))
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
  setCredentialMock.mockResolvedValue(undefined)
  importBackupMock.mockResolvedValue(undefined)
  confirmMock.mockResolvedValue(true)
})

afterEach(() => {
  document.documentElement.dataset.theme = ''
  setLocale('zh-CN')
})

describe('SettingsTab — 表单保护性行为', () => {
  it('修改字段后出现未保存 banner', async () => {
    const wrapper = mountSettings()

    expect(wrapper.find('.settings-banner').exists()).toBe(false)
    await wrapper.get('input[type="number"]').setValue(30)

    expect(wrapper.find('.settings-banner').exists()).toBe(true)
    expect(wrapper.find('.settings-banner').text()).toContain(t('settings.unsaved_banner', '1'))
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
      poll_interval_minutes: 25,
      proxy_mode: 'none',
      deepseek_enabled: true,
      deepseek_model: 'deepseek-v4-flash',
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
    // 真实 i18n：语言预览立即生效
    expect(getLocale()).toBe('en-US')

    await languageSelect.trigger('mouseleave')
    expect(getLocale()).toBe('zh-CN')
  })

  it('主题预览后离开恢复为当前表单主题', async () => {
    const wrapper = mountSettings(createSettings({ theme: 'light' }))
    await clickSidebar(wrapper, 'settings.appearance')

    const themeSelect = wrapper.findAll('.theme-select')[1]
    await themeSelect.get('.theme-select-trigger').trigger('click')
    const darkOption = themeSelect.findAll('.theme-select-option').find(option => option.text() === t('settings.theme_dark'))
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
    expect(setCredentialMock).not.toHaveBeenCalled()
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

describe('SettingsTab — 保存原子性（P0 #2）', () => {
  it('updateSettings 失败时不写入凭据，凭据输入与未设置标记保持不变', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_api_key_set: false, github_token_set: false }))
    await clickSidebar(wrapper, 'settings.ai')

    const apiKeyInput = wrapper.find('input[type="password"]')
    await apiKeyInput.setValue('sk-new-key')

    updateSettingsMock.mockRejectedValueOnce(new Error('db locked'))

    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(updateSettingsMock).toHaveBeenCalledOnce()
    // 凭据在 updateSettings 之后才写入；updateSettings 失败 → 凭据不应被调用
    expect(setCredentialMock).not.toHaveBeenCalled()
    // 输入未清空、标记未被误置为 true（避免“凭据已存但 UI 以为未存”的反向不一致）
    expect((apiKeyInput.element as HTMLInputElement).value).toBe('sk-new-key')
    // placeholder 仍为「未设置」提示（真实 i18n 文案）
    expect((apiKeyInput.element as HTMLInputElement).placeholder).toBe(t('settings.api_key_input'))
  })

  it('updateSettings 成功后才写入凭据（调用顺序：先主设置后凭据）', async () => {
    const wrapper = mountSettings(createSettings({ deepseek_api_key_set: false }))
    await clickSidebar(wrapper, 'settings.ai')

    await wrapper.find('input[type="password"]').setValue('sk-new-key')
    await wrapper.get('.setting-actions .btn-primary').trigger('click')
    await flushPromises()

    expect(updateSettingsMock).toHaveBeenCalledOnce()
    expect(setCredentialMock).toHaveBeenCalledWith('deepseek_api_key', 'sk-new-key')
    // 严格校验调用顺序：updateSettings 必须早于 setCredential
    const settingsOrder = vi.mocked(updateSettings).mock.invocationCallOrder[0]
    const credOrder = vi.mocked(setCredential).mock.invocationCallOrder[0]
    expect(settingsOrder).toBeLessThan(credOrder)
  })
})
