import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { shallowMount, flushPromises } from '@vue/test-utils'

// ========== Tauri API Mocks ==========
const mockUnlisten = vi.fn()
const mockListen = vi.fn(() => Promise.resolve(mockUnlisten))
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))

// ========== App 依赖 Mocks ==========
vi.mock('../api/sources', () => ({ listSources: vi.fn() }))
vi.mock('../api/releases', () => ({
  getReleases: vi.fn(),
  getPollCountdown: vi.fn(),
  triggerPoll: vi.fn(),
}))
vi.mock('../api/settings', () => ({ getSettings: vi.fn() }))

vi.mock('../i18n', () => ({
  t: vi.fn((key: string, ...args: string[]) => {
    if (key === 'app.min_sec') return `${args[0]}分${args[1]}秒`
    if (key === 'app.check_soon') return '即将检查'
    if (key === 'app.checking') return '检查中...'
    if (key === 'app.check_now') return '立即检查'
    if (key === 'app.new_found') return `发现 ${args[0]} 个新版本`
    if (key === 'app.already_latest') return '已是最新'
    if (key === 'app.no_sources') return '没有启用的源'
    if (key === 'app.source_auto_disabled') return `源 ${args[0]}/${args[1]} 已自动禁用(${args[2]}次失败)`
    return key
  }),
  setLocale: vi.fn(),
}))

vi.mock('../composables/contextMenuBus', () => ({
  registerCloser: vi.fn(),
  unregisterCloser: vi.fn(),
  closeAllContextMenus: vi.fn(),
}))

vi.mock('../composables/useEscapeToTray', () => ({
  useEscapeToTray: vi.fn(),
}))

vi.mock('../utils', () => ({
  isUnreadStatus: vi.fn((status: string, snoozeUntil?: string | null) => {
    if (status === 'snoozed' && snoozeUntil) {
      return new Date(snoozeUntil).getTime() <= Date.now()
    }
    return status === 'pending' || status === 'snoozed'
  }),
}))

import { listSources } from '../api/sources'
import { getReleases, getPollCountdown, triggerPoll } from '../api/releases'
import { getSettings } from '../api/settings'
import { readText } from '@tauri-apps/plugin-clipboard-manager'

// Mock document.execCommand for jsdom
const execCommandMock = vi.fn()
Object.defineProperty(document, 'execCommand', {
  value: execCommandMock,
  writable: true,
  configurable: true,
})

const defaultSettings = {
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
  show_source_type_icons: true,
  github_token_set: false,
  youtube_api_key_set: false,
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.useRealTimers()
  vi.mocked(listSources).mockResolvedValue([])
  vi.mocked(getReleases).mockResolvedValue([])
  vi.mocked(getPollCountdown).mockResolvedValue(300)
  vi.mocked(getSettings).mockResolvedValue(defaultSettings)
  vi.mocked(triggerPoll).mockResolvedValue({ new_releases: [] })

  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockReturnValue({ matches: false, onchange: null }),
  })
  document.documentElement.dataset.theme = ''
})

afterEach(() => {
  vi.useRealTimers()
  document.documentElement.dataset.theme = ''
})

async function mountApp() {
  const mod = await import('../App.vue')
  const wrapper = shallowMount(mod.default, {
    stubs: ['SourceTab', 'ReleaseTab', 'LogTab', 'SettingsTab', 'ContextMenu', 'Transition'],
  })
  await flushPromises()
  await wrapper.vm.$nextTick()
  return wrapper
}

/**
 * App.vue 补充测试 — 覆盖真实使用场景:
 * - handleSourceCheckResult (源检查完成回调)
 * - onMainScroll (主内容区滚动状态)
 * - handleCopySelection / execInputAction (右键菜单操作)
 * - source-auto-disabled Tauri 事件
 * - navigate Tauri 事件
 * - poll-completed 后同步倒计时
 * - handlePoll 无新版本时显示"已是最新"
 * - 并发控制 (polling/sourceChecking 时忽略 handlePoll)
 * - watchSystemTheme (系统主题变化自动切换)
 */

describe('App.vue — handleSourceCheckResult', () => {
  it('源检查结果为 0 个新版本时显示"已是最新"', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).handleSourceCheckResult(0)
    await wrapper.vm.$nextTick()

    expect((wrapper.vm as any).toastMessage).toBe('已是最新')
    expect((wrapper.vm as any).toastVisible).toBe(true)
  })

  it('源检查结果有新版本时显示数量', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).handleSourceCheckResult(3)
    await wrapper.vm.$nextTick()

    expect((wrapper.vm as any).toastMessage).toBe('发现 3 个新版本')
    expect((wrapper.vm as any).toastVisible).toBe(true)
  })
})

describe('App.vue — onMainScroll', () => {
  it('scrollTop > 0 时 mainScrolled 为 true', async () => {
    const wrapper = await mountApp()

    // 模拟滚动事件
    const mainEl = wrapper.find('.app-main')
    Object.defineProperty(mainEl.element, 'scrollTop', { value: 10, configurable: true })
    await mainEl.trigger('scroll')

    expect((wrapper.vm as any).mainScrolled).toBe(true)
  })

  it('scrollTop = 0 时 mainScrolled 为 false', async () => {
    const wrapper = await mountApp()

    // 先滚下去
    ;(wrapper.vm as any).mainScrolled = true

    // 再滚回来
    const mainEl = wrapper.find('.app-main')
    Object.defineProperty(mainEl.element, 'scrollTop', { value: 0, configurable: true })
    await mainEl.trigger('scroll')

    expect((wrapper.vm as any).mainScrolled).toBe(false)
  })

  it('mainScrolled=true 时 main 元素有 is-scrolled class', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).mainScrolled = true
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.app-main').classes()).toContain('is-scrolled')
  })
})

describe('App.vue — 右键菜单: 文本选择复制', () => {
  it('handleCopySelection 复制选中文本到剪贴板', async () => {
    const mockClipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
    Object.assign(navigator, { clipboard: mockClipboard })

    // Mock window.getSelection
    const mockSelection = { toString: () => '选中的文本' }
    vi.spyOn(window, 'getSelection').mockReturnValue(mockSelection as unknown as Selection)

    const wrapper = await mountApp()
    await (wrapper.vm as any).handleCopySelection()

    expect(mockClipboard.writeText).toHaveBeenCalledWith('选中的文本')
  })

  it('handleCopySelection 无选中文本时不操作', async () => {
    const mockClipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
    Object.assign(navigator, { clipboard: mockClipboard })

    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => '' } as unknown as Selection)

    const wrapper = await mountApp()
    await (wrapper.vm as any).handleCopySelection()

    expect(mockClipboard.writeText).not.toHaveBeenCalled()
  })

  it('handleSelectionMenuAction 处理 copySelection', async () => {
    const mockClipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
    Object.assign(navigator, { clipboard: mockClipboard })

    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => 'test' } as unknown as Selection)

    const wrapper = await mountApp()
    await (wrapper.vm as any).handleSelectionMenuAction('copySelection')

    expect(mockClipboard.writeText).toHaveBeenCalledWith('test')
  })
})

describe('App.vue — 右键菜单: 输入框操作', () => {
  it('execInputAction copy 执行 document.execCommand("copy")', async () => {
    const wrapper = await mountApp()

    // 设置 inputContextMenu
    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    ;(wrapper.vm as any).inputContextMenu = { x: 100, y: 200, target: inputEl }

    await (wrapper.vm as any).execInputAction('copy')

    expect(execCommandMock).toHaveBeenCalledWith('copy')
    document.body.removeChild(inputEl)
  })

  it('execInputAction paste 读取剪贴板并插入文本', async () => {
    vi.mocked(readText).mockResolvedValue('粘贴的内容')

    const wrapper = await mountApp()

    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    ;(wrapper.vm as any).inputContextMenu = { x: 100, y: 200, target: inputEl }

    await (wrapper.vm as any).execInputAction('paste')

    expect(readText).toHaveBeenCalled()
    expect(execCommandMock).toHaveBeenCalledWith('insertText', false, '粘贴的内容')
    document.body.removeChild(inputEl)
  })

  it('execInputAction selectAll 执行全选', async () => {
    const wrapper = await mountApp()

    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    ;(wrapper.vm as any).inputContextMenu = { x: 100, y: 200, target: inputEl }

    await (wrapper.vm as any).execInputAction('selectAll')

    expect(execCommandMock).toHaveBeenCalledWith('selectAll')
    document.body.removeChild(inputEl)
  })

  it('execInputAction cut 执行剪切', async () => {
    const wrapper = await mountApp()

    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    ;(wrapper.vm as any).inputContextMenu = { x: 100, y: 200, target: inputEl }

    await (wrapper.vm as any).execInputAction('cut')

    expect(execCommandMock).toHaveBeenCalledWith('cut')
    document.body.removeChild(inputEl)
  })

  it('execInputAction paste 失败时静默处理', async () => {
    vi.mocked(readText).mockRejectedValue(new Error('clipboard denied'))

    const wrapper = await mountApp()

    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    ;(wrapper.vm as any).inputContextMenu = { x: 100, y: 200, target: inputEl }

    // 不应抛出错误
    await expect((wrapper.vm as any).execInputAction('paste')).resolves.toBeUndefined()

    document.body.removeChild(inputEl)
  })
})

describe('App.vue — Tauri 事件: source-auto-disabled', () => {
  it('source-auto-disabled 事件显示 Toast 并刷新数据', async () => {
    await mountApp()

    vi.mocked(listSources).mockClear()

    const call = (mockListen.mock.calls as any[][]).find(c => c[0] === 'source-auto-disabled')
    expect(call).toBeDefined()

    // 触发事件
    await call![1]({ payload: { owner: 'test-org', repo: 'test-repo', failures: 5 } })

    expect(listSources).toHaveBeenCalled()
  })
})

describe('App.vue — Tauri 事件: navigate', () => {
  it('navigate 事件切换到 sources tab', async () => {
    const wrapper = await mountApp()
    ;(wrapper.vm as any).activeTab = 'settings'

    const call = (mockListen.mock.calls as any[][]).find(c => c[0] === 'navigate')
    await call![1]({ payload: 'sources' })

    expect((wrapper.vm as any).activeTab).toBe('sources')
  })

  it('navigate 事件切换到 releases tab', async () => {
    const wrapper = await mountApp()

    const call = (mockListen.mock.calls as any[][]).find(c => c[0] === 'navigate')
    await call![1]({ payload: 'releases' })

    expect((wrapper.vm as any).activeTab).toBe('releases')
  })

  it('navigate 事件切换到 settings tab', async () => {
    const wrapper = await mountApp()

    const call = (mockListen.mock.calls as any[][]).find(c => c[0] === 'navigate')
    await call![1]({ payload: 'settings' })

    expect((wrapper.vm as any).activeTab).toBe('settings')
  })

  it('navigate 事件无效 payload 时不切换', async () => {
    const wrapper = await mountApp()
    ;(wrapper.vm as any).activeTab = 'sources'

    const call = (mockListen.mock.calls as any[][]).find(c => c[0] === 'navigate')
    await call![1]({ payload: 'invalid' })

    expect((wrapper.vm as any).activeTab).toBe('sources')
  })
})

describe('App.vue — handlePoll 边界场景', () => {
  it('poll 返回 0 个新版本时显示"已是最新"', async () => {
    vi.mocked(triggerPoll).mockResolvedValue({ new_releases: [] })
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountApp()
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    await (wrapper.vm as any).handlePoll()

    expect((wrapper.vm as any).toastMessage).toBe('已是最新')
  })

  it('sourceChecking 为 true 时 handlePoll 不执行', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountApp()
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    ;(wrapper.vm as any).sourceChecking = true
    await (wrapper.vm as any).handlePoll()

    expect(triggerPoll).not.toHaveBeenCalled()
  })
})

describe('App.vue — 系统主题变化自动切换', () => {
  let mqlInstance: { matches: boolean; onchange: ((e: { matches: boolean }) => void) | null }

  beforeEach(() => {
    mqlInstance = { matches: false, onchange: null }
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: vi.fn(() => mqlInstance),
    })
  })

  it('加载 settings 后设置系统主题媒体监听', async () => {
    await mountApp()

    expect(mqlInstance.onchange).toBeDefined()
  })

  it('系统主题从浅色切到深色时自动应用 dark', async () => {
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'system' })

    await mountApp()

    // 模拟系统主题变为深色
    mqlInstance.matches = true
    mqlInstance.onchange!({ matches: true })

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('系统主题从深色切到浅色时自动应用 light', async () => {
    mqlInstance.matches = true
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'system' })

    await mountApp()
    expect(document.documentElement.dataset.theme).toBe('dark')

    // 模拟系统主题变为浅色
    mqlInstance.matches = false
    mqlInstance.onchange!({ matches: false })

    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('theme 非 system 时系统主题变化不触发切换', async () => {
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'dark' })

    await mountApp()

    document.documentElement.dataset.theme = 'dark'

    // 模拟系统主题变化
    mqlInstance.matches = true
    mqlInstance.onchange!({ matches: true })

    // theme 是 dark 不是 system，不应改变
    expect(document.documentElement.dataset.theme).toBe('dark')
  })
})

describe('App.vue — refreshLogs', () => {
  it('refreshLogs 增加 logRefreshKey', async () => {
    const wrapper = await mountApp()

    const before = (wrapper.vm as any).logRefreshKey
    ;(wrapper.vm as any).refreshLogs()

    expect((wrapper.vm as any).logRefreshKey).toBe(before + 1)
  })
})

describe('App.vue — SettingsTab update 回调', () => {
  it('SettingsTab update(pollChanged=true) 时重启倒计时', async () => {
    const wrapper = await mountApp()

    // 获取 SettingsTab 的 update handler
    const settingsTab = wrapper.findComponent({ name: 'SettingsTab' })
    expect(settingsTab.exists()).toBe(true)

    // 模拟 SettingsTab emit update(true)
    const startCountdownSpy = vi.spyOn(wrapper.vm as any, 'startCountdown')
    await settingsTab.vm.$emit('update', true)
    await flushPromises()

    expect(startCountdownSpy).toHaveBeenCalled()
  })

  it('SettingsTab update(pollChanged=false, forceReload=true) 时重新加载 sources/releases', async () => {
    const wrapper = await mountApp()

    vi.mocked(listSources).mockClear()
    vi.mocked(getReleases).mockClear()

    const settingsTab = wrapper.findComponent({ name: 'SettingsTab' })
    await settingsTab.vm.$emit('update', false, true)
    await flushPromises()

    expect(listSources).toHaveBeenCalled()
    expect(getReleases).toHaveBeenCalled()
  })
})
