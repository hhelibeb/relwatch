import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { shallowMount, flushPromises } from '@vue/test-utils'

// ========== Tauri API Mocks ==========
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))

// listen 返回 mockUnlisten，用于验证清理
const mockUnlisten = vi.fn()
const mockListen = vi.fn(() => Promise.resolve(mockUnlisten))
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))

vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))

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
    if (key === 'app.load_failed') return `加载失败:${args.join(',')}`
    if (key === 'app.check_failed') return `检查失败:${args.join(',')}`
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
import { useEscapeToTray } from '../composables/useEscapeToTray'
import { registerCloser, unregisterCloser } from '../composables/contextMenuBus'

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
  vi.mocked(listSources).mockResolvedValue([])
  vi.mocked(getReleases).mockResolvedValue([])
  vi.mocked(getPollCountdown).mockResolvedValue(300)
  vi.mocked(getSettings).mockResolvedValue(defaultSettings)
  vi.mocked(triggerPoll).mockResolvedValue({ new_releases: [] })

  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockReturnValue({ matches: false }),
  })
  document.documentElement.dataset.theme = ''
})

afterEach(() => {
  vi.useRealTimers()
  document.documentElement.dataset.theme = ''
})

/**
 * App.vue 完整运行场景测试
 *
 * 根组件：初始化加载、Tab 导航、Toast 系统、主题管理、倒计时与轮询、
 *   Tauri 事件监听、右键菜单管理、useEscapeToTray 集成。
 */
async function mountApp() {
  const mod = await import('../App.vue')
  const wrapper = shallowMount(mod.default, {
    stubs: ['SourceTab', 'ReleaseTab', 'LogTab', 'SettingsTab', 'ContextMenu', 'Transition'],
  })
  await flushPromises()
  await wrapper.vm.$nextTick()
  return wrapper
}

describe('App.vue — 挂载初始化', () => {
  it('onMounted 时同时加载 sources / releases / settings', async () => {
    await mountApp()

    expect(listSources).toHaveBeenCalled()
    expect(getReleases).toHaveBeenCalled()
    expect(getSettings).toHaveBeenCalled()
  })

  it('某个 API 失败时不影响其他 API（Promise.allSettled）', async () => {
    vi.mocked(listSources).mockRejectedValue(new Error('网络错误'))
    vi.mocked(getReleases).mockResolvedValue([{ id: 1, tag_name: 'v1' } as never])

    await mountApp()

    // releases 正常返回
    expect(getReleases).toHaveBeenCalled()
    expect(getSettings).toHaveBeenCalled()
  })

  it('加载 settings 后更新语言和主题', async () => {
    vi.mocked(getSettings).mockResolvedValue({
      ...defaultSettings,
      language: 'en-US',
      theme: 'dark',
    })

    const { setLocale } = await import('../i18n')
    await mountApp()

    expect(setLocale).toHaveBeenCalledWith('en-US')
    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('注册 useEscapeToTray', async () => {
    await mountApp()

    expect(useEscapeToTray).toHaveBeenCalled()
  })

  it('注册 4 个 Tauri 事件监听器', async () => {
    await mountApp()

    expect(mockListen).toHaveBeenCalledWith('navigate', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('poll-completed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('release-state-changed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('source-auto-disabled', expect.any(Function))
  })

  it('注册 contextMenuBus 和 document 事件', async () => {
    await mountApp()

    expect(registerCloser).toHaveBeenCalled()
  })

  it('倒计时同步调用了 getPollCountdown', async () => {
    await mountApp()

    expect(getPollCountdown).toHaveBeenCalled()
  })
})

describe('App.vue — Tab 切换', () => {
  it('默认显示 Sources', async () => {
    const wrapper = await mountApp()

    expect((wrapper.vm as any).activeTab).toBe('sources')
  })

  it('点击 Releases 标签', async () => {
    const wrapper = await mountApp()

    await wrapper.findAll('nav.tabs button')[1].trigger('click')

    expect((wrapper.vm as any).activeTab).toBe('releases')
  })

  it('点击 Logs 标签', async () => {
    const wrapper = await mountApp()

    await wrapper.findAll('nav.tabs button')[2].trigger('click')

    expect((wrapper.vm as any).activeTab).toBe('logs')
  })

  it('点击 Settings 标签', async () => {
    const wrapper = await mountApp()

    await wrapper.findAll('nav.tabs button')[3].trigger('click')

    expect((wrapper.vm as any).activeTab).toBe('settings')
  })
})

describe('App.vue — Toast 系统', () => {
  it('showToast 设置消息并显示', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).showToast('测试消息')
    await wrapper.vm.$nextTick()

    expect((wrapper.vm as any).toastVisible).toBe(true)
    expect((wrapper.vm as any).toastMessage).toBe('测试消息')
    expect(wrapper.find('.toast').exists()).toBe(true)
  })

  it('Toast 3 秒后自动隐藏', async () => {
    vi.useFakeTimers()
    const wrapper = await mountApp()

    ;(wrapper.vm as any).showToast('自动消失')
    expect((wrapper.vm as any).toastVisible).toBe(true)

    vi.advanceTimersByTime(3000)

    expect((wrapper.vm as any).toastVisible).toBe(false)
    vi.useRealTimers()
  })

  it('连续 showToast 进入队列，旧消息消失后依次显示', async () => {
    vi.useFakeTimers()
    const wrapper = await mountApp()

    ;(wrapper.vm as any).showToast('第一条')
    vi.advanceTimersByTime(2000)

    ;(wrapper.vm as any).showToast('第二条')
    // 第二条入队，第一条仍继续显示
    expect((wrapper.vm as any).toastMessage).toBe('第一条')

    // 第一条满 3 秒后隐藏，经离场间隙后第二条显示
    vi.advanceTimersByTime(1000)
    expect((wrapper.vm as any).toastVisible).toBe(false)
    vi.advanceTimersByTime(350)
    expect((wrapper.vm as any).toastVisible).toBe(true)
    expect((wrapper.vm as any).toastMessage).toBe('第二条')

    vi.advanceTimersByTime(3000)
    expect((wrapper.vm as any).toastVisible).toBe(false)
    vi.useRealTimers()
  })

  it('鼠标悬浮时 Toast 不消失，移开后重新计时', async () => {
    vi.useFakeTimers()
    const wrapper = await mountApp()

    ;(wrapper.vm as any).showToast('悬浮暂停')
    await wrapper.vm.$nextTick()
    vi.advanceTimersByTime(1000)

    await wrapper.find('.toast').trigger('mouseenter')
    // 悬浮期间远超过 3 秒也不消失
    vi.advanceTimersByTime(10000)
    expect((wrapper.vm as any).toastVisible).toBe(true)

    await wrapper.find('.toast').trigger('mouseleave')
    // 移开后重新计时 3 秒
    vi.advanceTimersByTime(2000)
    expect((wrapper.vm as any).toastVisible).toBe(true)
    vi.advanceTimersByTime(1000)
    expect((wrapper.vm as any).toastVisible).toBe(false)
    vi.useRealTimers()
  })
})

describe('App.vue — 主题管理', () => {
  it('dark 主题', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).applyTheme('dark')

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('light 主题', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).applyTheme('light')

    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('system 主题根据 prefers-color-scheme 决定', async () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: true })
    const wrapper = await mountApp()

    ;(wrapper.vm as any).applyTheme('system')

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('system 主题浅色模式', async () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: false })
    const wrapper = await mountApp()

    ;(wrapper.vm as any).applyTheme('system')

    expect(document.documentElement.dataset.theme).toBe('light')
  })
})

describe('App.vue — Poll 轮询', () => {
  it('无启用源时 handlePoll 不 triggerPoll 并显示 Toast', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountApp()
    // 重新加载 sources
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    await (wrapper.vm as any).handlePoll()

    expect(triggerPoll).not.toHaveBeenCalled()
    expect((wrapper.vm as any).toastVisible).toBe(true)
  })

  it('有启用源时 handlePoll 调用 triggerPoll', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountApp()
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    await (wrapper.vm as any).handlePoll()

    expect(triggerPoll).toHaveBeenCalled()
  })

  it('轮询中按钮 disabled（防止重复点击）', async () => {
    vi.mocked(triggerPoll).mockReturnValue(new Promise(() => {}))
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountApp()
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    ;(wrapper.vm as any).handlePoll() // 不 await 让 polling 保持 true
    await wrapper.vm.$nextTick()

    expect((wrapper.vm as any).polling).toBe(true)
    const checkBtn = wrapper.find('.btn-primary')
    expect((checkBtn.element as HTMLButtonElement).disabled).toBe(true)
  })

  it('有新版本时 Toast 显示数量', async () => {
    vi.mocked(triggerPoll).mockResolvedValue({
      new_releases: [{ id: 1, tag_name: 'v2.0.0' }] as never,
    })
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountApp()
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    await (wrapper.vm as any).handlePoll()

    expect((wrapper.vm as any).toastMessage).toBe('发现 1 个新版本')
    expect((wrapper.vm as any).toastVisible).toBe(true)
  })
})

describe('App.vue — 版本统计 computed', () => {
  it('repoKey 统一小写', async () => {
    const wrapper = await mountApp()

    expect((wrapper.vm as any).repoKey('Tauri-Apps', 'Tauri')).toBe('tauri-apps/tauri')
    expect((wrapper.vm as any).repoKey('VueJS', 'Core')).toBe('vuejs/core')
  })

  it('unreadReleaseCounts 按 repo 统计未读', async () => {
    const { isUnreadStatus } = await import('../utils')
    vi.mocked(isUnreadStatus).mockImplementation((status: string) => status === 'pending')
    vi.mocked(getReleases).mockResolvedValue([
      { id: 1, owner: 'tauri-apps', repo: 'tauri', notification_status: 'pending', snooze_until: null } as never,
      { id: 2, owner: 'tauri-apps', repo: 'tauri', notification_status: 'pending', snooze_until: null } as never,
      { id: 3, owner: 'vuejs', repo: 'core', notification_status: 'clicked', snooze_until: null } as never,
    ])

    const wrapper = await mountApp()
    // 触发 computed 重新计算
    await flushPromises()
    await new Promise(resolve => setTimeout(resolve, 20))

    const counts = (wrapper.vm as any).unreadReleaseCounts
    expect(counts['tauri-apps/tauri']).toBe(2)
    expect(counts['vuejs/core']).toBeUndefined()
  })

  it('totalReleaseCounts 按 repo 统计总数', async () => {
    vi.mocked(getReleases).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', notification_status: 'pending' } as never,
      { id: 2, owner: 'a', repo: 'b', notification_status: 'clicked' } as never,
      { id: 3, owner: 'c', repo: 'd', notification_status: 'ignored' } as never,
    ])

    const wrapper = await mountApp()
    await flushPromises()
    await new Promise(resolve => setTimeout(resolve, 20))

    const counts = (wrapper.vm as any).totalReleaseCounts
    expect(counts['a/b']).toBe(2)
    expect(counts['c/d']).toBe(1)
  })
})

describe('App.vue — 跨 Tab 导航', () => {
  it('openSourceReleases 跳转到 releases 并设搜索词', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).openSourceReleases('tauri-apps/tauri')

    expect((wrapper.vm as any).activeTab).toBe('releases')
    expect((wrapper.vm as any).releaseSearch).toBe('tauri-apps/tauri')
    expect((wrapper.vm as any).releaseStatusFilter).toBe('all')
  })

  it('openSourceUnreadReleases 跳转并设未读筛选', async () => {
    const wrapper = await mountApp()

    ;(wrapper.vm as any).openSourceUnreadReleases('vuejs/core')

    expect((wrapper.vm as any).activeTab).toBe('releases')
    expect((wrapper.vm as any).releaseSearch).toBe('vuejs/core')
    expect((wrapper.vm as any).releaseStatusFilter).toBe('unread')
  })
})

describe('App.vue — 卸载清理', () => {
  it('卸载时调用 unlisten 清理', async () => {
    const wrapper = await mountApp()

    wrapper.unmount()

    expect(mockUnlisten).toHaveBeenCalled()
  })

  it('卸载时注销 contextMenuBus', async () => {
    const wrapper = await mountApp()

    wrapper.unmount()

    expect(unregisterCloser).toHaveBeenCalled()
  })

  it('卸载时清理 countdown 定时器（fake timers）', async () => {
    vi.useFakeTimers()
    const clearIntervalSpy = vi.spyOn(globalThis, 'clearInterval')
    const wrapper = await mountApp()

    wrapper.unmount()

    // onMounted 的 startCountdown 会 setInterval，卸载时应 clearInterval
    expect(clearIntervalSpy).toHaveBeenCalled()
    vi.useRealTimers()
    clearIntervalSpy.mockRestore()
  })

  it('卸载时清理 toast 定时器', async () => {
    vi.useFakeTimers()
    const clearTimeoutSpy = vi.spyOn(globalThis, 'clearTimeout')
    const wrapper = await mountApp()

    // 触发一个 toast，创建 setTimeout
    ;(wrapper.vm as any).showToast('msg')
    wrapper.unmount()

    // toast setTimeout 应在卸载时被清理
    expect(clearTimeoutSpy).toHaveBeenCalled()
    vi.useRealTimers()
    clearTimeoutSpy.mockRestore()
  })

  it('卸载时清理 systemThemeMedia.onchange', async () => {
    const onchangeSpy = vi.fn()
    window.matchMedia = vi.fn().mockReturnValue({ matches: false, onchange: onchangeSpy })
    const wrapper = await mountApp()

    wrapper.unmount()

    // watchSystemTheme 赋了 onchange，卸载时应置 null
    expect((window.matchMedia as any)()).toHaveProperty('onchange', null)
  })
})

describe('App.vue — 异步错误处理（P1 #4）', () => {
  it('loadSources 失败时显示 toast 且不产生未捕获 rejection', async () => {
    vi.mocked(listSources).mockRejectedValue(new Error('db locked'))
    const wrapper = await mountApp()
    await flushPromises()

    expect((wrapper.vm as any).toastVisible).toBe(true)
    expect((wrapper.vm as any).toastMessage).toContain('加载失败')
    expect((wrapper.vm as any).toastMessage).toContain('db locked')
  })

  it('loadReleases 失败时显示 toast', async () => {
    vi.mocked(getReleases).mockRejectedValue(new Error('network'))
    const wrapper = await mountApp()
    await flushPromises()

    expect((wrapper.vm as any).toastVisible).toBe(true)
    expect((wrapper.vm as any).toastMessage).toContain('加载失败')
    expect((wrapper.vm as any).toastMessage).toContain('network')
  })

  it('loadSettings 失败时显示 toast', async () => {
    vi.mocked(getSettings).mockRejectedValue(new Error('settings io'))
    const wrapper = await mountApp()
    await flushPromises()

    expect((wrapper.vm as any).toastVisible).toBe(true)
    expect((wrapper.vm as any).toastMessage).toContain('加载失败')
    expect((wrapper.vm as any).toastMessage).toContain('settings io')
  })

  it('handlePoll 失败时显示检查失败 toast 且 polling 复位', async () => {
    vi.mocked(triggerPoll).mockRejectedValue(new Error('poll err'))
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountApp()
    await (wrapper.vm as any).loadSources()
    await flushPromises()

    await (wrapper.vm as any).handlePoll()

    expect((wrapper.vm as any).toastVisible).toBe(true)
    expect((wrapper.vm as any).toastMessage).toContain('检查失败')
    expect((wrapper.vm as any).toastMessage).toContain('poll err')
    expect((wrapper.vm as any).polling).toBe(false)
  })
})

describe('App.vue — Tauri 事件处理', () => {
  it('poll-completed 触发数据刷新', async () => {
    await mountApp()

    vi.mocked(listSources).mockClear()
    vi.mocked(getReleases).mockClear()

    const pollCall = (mockListen.mock.calls as any[][]).find(c => c[0] === 'poll-completed')
    expect(pollCall).toBeDefined()

    await pollCall![1]({ payload: {} })

    expect(listSources).toHaveBeenCalled()
    expect(getReleases).toHaveBeenCalled()
  })

  it('release-state-changed 触发 releases 刷新', async () => {
    await mountApp()

    vi.mocked(getReleases).mockClear()

    const stateCall = (mockListen.mock.calls as any[][]).find(c => c[0] === 'release-state-changed')
    expect(stateCall).toBeDefined()

    await stateCall![1]({ payload: {} })

    expect(getReleases).toHaveBeenCalled()
  })
})

describe('App.vue — 倒计时', () => {
  it('syncCountdown 调用 getPollCountdown 并格式化', async () => {
    vi.mocked(getPollCountdown).mockResolvedValue(125)
    const wrapper = await mountApp()

    await (wrapper.vm as any).syncCountdown()

    expect(getPollCountdown).toHaveBeenCalled()
    expect((wrapper.vm as any).countdown).toBe('2分5秒')
  })

  it('倒计时为 0 时显示"即将检查"', async () => {
    vi.mocked(getPollCountdown).mockResolvedValue(0)
    const wrapper = await mountApp()

    await (wrapper.vm as any).syncCountdown()

    expect((wrapper.vm as any).countdown).toBe('即将检查')
  })

  it('formatCountdown 正数时分秒，0 或负数 "即将检查"', async () => {
    const wrapper = await mountApp()

    expect((wrapper.vm as any).formatCountdown(150)).toBe('2分30秒')
    expect((wrapper.vm as any).formatCountdown(60)).toBe('1分0秒')
    expect((wrapper.vm as any).formatCountdown(0)).toBe('即将检查')
    expect((wrapper.vm as any).formatCountdown(-1)).toBe('即将检查')
  })
})
