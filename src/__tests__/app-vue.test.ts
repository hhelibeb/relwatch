import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { t, setLocale } from '../i18n'
import { defaultSettings } from './helpers'

// ========== Tauri 边界 Mocks（应用内模块一律走真实实现） ==========
const { mockUnlisten, mockListen, mockWindow } = vi.hoisted(() => {
  const mockUnlisten = vi.fn()
  const mockListen = vi.fn((_event: string, _handler: (...args: unknown[]) => void) => Promise.resolve(mockUnlisten))
  const mockWindow = {
    innerSize: vi.fn(),
    scaleFactor: vi.fn(),
    setSize: vi.fn(),
    isMaximized: vi.fn().mockResolvedValue(false),
    outerPosition: vi.fn().mockResolvedValue({ x: 0, y: 0 }),
    onResized: vi.fn().mockResolvedValue(mockUnlisten),
  }
  return { mockUnlisten, mockListen, mockWindow }
})
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => mockWindow,
  LogicalSize: class {
    constructor(public width: number, public height: number) {}
  },
}))
// Agent 启用配置：默认开启（App.vue 挂载时读取；工作区面板开关依赖它）
vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    getAgentConfig: vi.fn().mockResolvedValue({
      enabled: true,
      agent_type: 'pi',
      binary: null,
      model: null,
      prompt_suffix: null,
      timeout_seconds: 300,
      skills: [],
    }),
  }
})

// API 层：仅替换发起 IPC 的函数，其余导出（sourceRepoKey 等注册表函数）保留真实实现
vi.mock('../api/sources', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/sources')>()
  return { ...actual, listSources: vi.fn() }
})
vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return { ...actual, getReleases: vi.fn(), getPollCountdown: vi.fn(), triggerPoll: vi.fn() }
})
vi.mock('../api/settings', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/settings')>()
  return { ...actual, getSettings: vi.fn() }
})

// 统计上报（内存聚合 + 5s 定时刷库）与托盘集成在测试中置空，避免定时器泄漏
vi.mock('../composables/useUsageTracking', () => ({
  track: vi.fn(),
  setUsageTrackingEnabled: vi.fn(),
  flushUsageTrackingNow: vi.fn(),
}))
vi.mock('../composables/useEscapeToTray', () => ({ useEscapeToTray: vi.fn() }))

import { listSources } from '../api/sources'
import { getReleases, getPollCountdown, triggerPoll } from '../api/releases'
import { getSettings } from '../api/settings'
import { invoke } from '@tauri-apps/api/core'
import { useEscapeToTray } from '../composables/useEscapeToTray'
import { closeAllContextMenus } from '../composables/contextMenuBus'

// ========== 子组件自定义 stub：通过真实 DOM 点击驱动 emit（事件入口） ==========
const SourceTabStub = defineComponent({
  name: 'SourceTab',
  props: ['sources', 'polling', 'unreadReleaseCounts', 'totalReleaseCounts', 'showSourceTypeIcons'],
  emits: ['update', 'check-result', 'check-busy', 'open-releases', 'open-unread-releases'],
  template: `
    <div class="stub-sourcetab">
      <button class="stub-check-result" @click="$emit('check-result', 2)">check-result</button>
      <button class="stub-check-busy" @click="$emit('check-busy', true)">check-busy</button>
      <button class="stub-open-releases" @click="$emit('open-releases', 'tauri-apps/tauri')">open-releases</button>
      <button class="stub-open-unread" @click="$emit('open-unread-releases', 'vuejs/core')">open-unread</button>
    </div>`,
})

const ReleaseTabStub = defineComponent({
  name: 'ReleaseTab',
  props: ['search', 'statusFilter'],
  emits: ['update'],
  template:
    '<div class="stub-releasetab"><span class="stub-search">{{ search }}</span><span class="stub-status">{{ statusFilter }}</span></div>',
})

const LogTabStub = defineComponent({
  name: 'LogTab',
  props: ['refreshKey'],
  emits: ['update'],
  template: '<div class="stub-logtab"><span class="stub-refresh-key">{{ refreshKey }}</span></div>',
})

const SettingsTabStub = defineComponent({
  name: 'SettingsTab',
  props: ['settings'],
  emits: ['update'],
  template: `
    <div class="stub-settingstab">
      <button class="stub-settings-restart" @click="$emit('update', true, false)">restart-countdown</button>
      <button class="stub-settings-reload" @click="$emit('update', false, true)">reload-all</button>
    </div>`,
})

const ContextMenuStub = defineComponent({
  name: 'ContextMenu',
  props: { x: Number, y: Number, items: Array },
  emits: ['action', 'close'],
  template: `
    <div class="stub-context-menu">
      <button v-for="item in items" :key="item.id" class="stub-menu-item" @click="$emit('action', item.id)">{{ item.label }}</button>
    </div>`,
})

const TransitionStub = defineComponent({
  name: 'Transition',
  template: '<div><slot /></div>',
})

// 真实 App.vue 挂载（子组件用上述 stub 替换，交互走事件入口）
async function mountRealApp() {
  const mod = await import('../App.vue')
  const wrapper = mount(mod.default, {
    global: {
      stubs: {
        SourceTab: SourceTabStub,
        ReleaseTab: ReleaseTabStub,
        LogTab: LogTabStub,
        SettingsTab: SettingsTabStub,
        ContextMenu: ContextMenuStub,
        Transition: TransitionStub,
        AgentWorkspace: true,
      },
    },
  })
  await flushPromises()
  await wrapper.vm.$nextTick()
  return wrapper
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
    value: vi.fn().mockReturnValue({ matches: false, onchange: null }),
  })
  // 默认：非最大化（最大化测试会覆盖，beforeEach 复位防残留串扰）
  mockWindow.isMaximized.mockResolvedValue(false)
  document.documentElement.dataset.theme = ''
  setLocale('zh-CN')
})

afterEach(() => {
  vi.useRealTimers()
  document.documentElement.dataset.theme = ''
  setLocale('zh-CN')
})

describe('App.vue — 挂载初始化', () => {
  it('onMounted 时同时加载 sources / releases / settings', async () => {
    await mountRealApp()

    expect(listSources).toHaveBeenCalled()
    expect(getReleases).toHaveBeenCalled()
    expect(getSettings).toHaveBeenCalled()
  })

  it('某个 API 失败时不影响其他 API（Promise.allSettled）', async () => {
    vi.mocked(listSources).mockRejectedValue(new Error('网络错误'))
    vi.mocked(getReleases).mockResolvedValue([{ id: 1, tag_name: 'v1' } as never])

    await mountRealApp()

    expect(getReleases).toHaveBeenCalled()
    expect(getSettings).toHaveBeenCalled()
  })

  it('加载 settings 后更新语言（真实 i18n）和主题', async () => {
    const { getLocale } = await import('../i18n')
    vi.mocked(getSettings).mockResolvedValue({
      ...defaultSettings,
      language: 'en-US',
      theme: 'dark',
    })

    await mountRealApp()

    expect(getLocale()).toBe('en-US')
    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('注册 useEscapeToTray', async () => {
    await mountRealApp()

    expect(useEscapeToTray).toHaveBeenCalled()
  })

  it('注册 4 个 Tauri 事件监听器', async () => {
    await mountRealApp()

    expect(mockListen).toHaveBeenCalledWith('navigate', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('poll-completed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('release-state-changed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('source-auto-disabled', expect.any(Function))
  })

  it('注册 contextMenuBus：菜单可被总线统一关闭', async () => {
    const wrapper = await mountRealApp()

    // 通过 document 右键事件打开输入框菜单（走真实注册链路）
    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    inputEl.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 20 }))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.stub-context-menu').exists()).toBe(true)

    // 总线关闭所有菜单 → App 的菜单关闭（真实 registerCloser 链路）
    closeAllContextMenus()
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.stub-context-menu').exists()).toBe(false)
    document.body.removeChild(inputEl)
  })

  it('倒计时同步调用了 getPollCountdown', async () => {
    await mountRealApp()

    expect(getPollCountdown).toHaveBeenCalled()
  })

  it('HMR 场景：重复挂载/卸载不会累积监听器', async () => {
    const mod = await import('../App.vue')
    const stubs = {
      SourceTab: SourceTabStub,
      ReleaseTab: ReleaseTabStub,
      LogTab: LogTabStub,
      SettingsTab: SettingsTabStub,
      ContextMenu: ContextMenuStub,
      Transition: TransitionStub,
    }
    const wrapper1 = mount(mod.default, { global: { stubs } })
    await flushPromises()
    wrapper1.unmount()

    const wrapper2 = mount(mod.default, { global: { stubs } })
    await flushPromises()
    wrapper2.unmount()

    // 两次挂载共 8 次 listen，全部 unlisten 都被调用（无泄漏）
    expect(mockListen).toHaveBeenCalledTimes(8)
    expect(mockUnlisten).toHaveBeenCalledTimes(8)
  })
})

describe('App.vue — Tab 切换', () => {
  it('默认显示 Sources tab', async () => {
    const wrapper = await mountRealApp()

    expect(wrapper.findAll('nav.tabs button')[0].classes()).toContain('active')
  })

  it('点击标签切换 tab（DOM 入口）', async () => {
    const wrapper = await mountRealApp()
    const buttons = wrapper.findAll('nav.tabs button')

    await buttons[1].trigger('click')
    expect(buttons[1].classes()).toContain('active')
    expect(buttons[0].classes()).not.toContain('active')

    await buttons[2].trigger('click')
    expect(buttons[2].classes()).toContain('active')

    await buttons[3].trigger('click')
    expect(buttons[3].classes()).toContain('active')
  })
})

describe('App.vue — Toast 系统', () => {
  it('无启用源时点击「立即检查」显示 Toast（真实文案）', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.no_sources'))
  })

  it('有新版本时 Toast 显示数量', async () => {
    vi.mocked(triggerPoll).mockResolvedValue({
      new_releases: [{ id: 1, tag_name: 'v2.0.0' } as never],
    })
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.new_found', '1'))
  })

  it('Toast 3 秒后自动隐藏', async () => {
    vi.useFakeTimers()
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(wrapper.find('.toast').exists()).toBe(true)

    vi.advanceTimersByTime(3000)
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.toast').exists()).toBe(false)
  })

  it('连续 Toast 进入队列，旧消息消失后依次显示', async () => {
    vi.useFakeTimers()
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    vi.mocked(triggerPoll).mockResolvedValueOnce({
      new_releases: [{ id: 1, tag_name: 'v1' }] as never,
    })
    vi.mocked(triggerPoll).mockResolvedValueOnce({
      new_releases: [{ id: 1, tag_name: 'v2' }, { id: 2, tag_name: 'v3' }] as never,
    })
    const wrapper = await mountRealApp()

    // 第一次检查 → 发现 1 个新版本
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(wrapper.find('.toast').text()).toBe(t('app.new_found', '1'))

    vi.advanceTimersByTime(2000)
    // 第二次检查 → 发现 2 个新版本（入队）
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(wrapper.find('.toast').text()).toBe(t('app.new_found', '1'))

    // 第一条满 3 秒隐藏，经离场间隙后第二条显示
    vi.advanceTimersByTime(1000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(false)
    vi.advanceTimersByTime(350)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').text()).toBe(t('app.new_found', '2'))

    vi.advanceTimersByTime(3000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(false)
  })

  it('鼠标悬浮时 Toast 不消失，移开后重新计时', async () => {
    vi.useFakeTimers()
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    vi.advanceTimersByTime(1000)

    await wrapper.find('.toast').trigger('mouseenter')
    vi.advanceTimersByTime(10000)
    expect(wrapper.find('.toast').exists()).toBe(true)

    await wrapper.find('.toast').trigger('mouseleave')
    // 移开后重新计时 3 秒
    vi.advanceTimersByTime(2000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(true)
    vi.advanceTimersByTime(1000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(false)
  })
})

describe('App.vue — 主题管理', () => {
  it('dark 主题设置后应用到 document', async () => {
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'dark' })
    await mountRealApp()

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('light 主题设置后应用到 document', async () => {
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'light' })
    await mountRealApp()

    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('system 主题根据 prefers-color-scheme 决定（深色系统）', async () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: true, onchange: null })
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'system' })
    await mountRealApp()

    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('system 主题浅色系统', async () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: false, onchange: null })
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'system' })
    await mountRealApp()

    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('系统主题变化自动切换（仅 theme=system 时）', async () => {
    const mqlInstance: { matches: boolean; onchange: ((e: { matches: boolean }) => void) | null } = { matches: false, onchange: null }
    window.matchMedia = vi.fn(() => mqlInstance) as never
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'system' })

    await mountRealApp()
    expect(document.documentElement.dataset.theme).toBe('light')
    expect(mqlInstance.onchange).toBeDefined()

    // 系统切到深色
    mqlInstance.matches = true
    mqlInstance.onchange!({ matches: true })
    expect(document.documentElement.dataset.theme).toBe('dark')

    // 系统切回浅色
    mqlInstance.matches = false
    mqlInstance.onchange!({ matches: false })
    expect(document.documentElement.dataset.theme).toBe('light')
  })

  it('theme 非 system 时系统主题变化不触发切换', async () => {
    const mqlInstance: { matches: boolean; onchange: ((e: { matches: boolean }) => void) | null } = { matches: false, onchange: null }
    window.matchMedia = vi.fn(() => mqlInstance) as never
    vi.mocked(getSettings).mockResolvedValue({ ...defaultSettings, theme: 'dark' })

    await mountRealApp()
    document.documentElement.dataset.theme = 'dark'

    mqlInstance.matches = true
    mqlInstance.onchange!({ matches: true })

    expect(document.documentElement.dataset.theme).toBe('dark')
  })
})

describe('App.vue — Poll 轮询', () => {
  it('无启用源时不触发 triggerPoll 并显示 Toast', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(triggerPoll).not.toHaveBeenCalled()
    expect(wrapper.find('.toast').exists()).toBe(true)
  })

  it('有启用源时触发 triggerPoll', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(triggerPoll).toHaveBeenCalled()
  })

  it('轮询中按钮 disabled（防止重复点击）', async () => {
    vi.mocked(triggerPoll).mockReturnValue(new Promise(() => {}))
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountRealApp()

    wrapper.find('.btn-primary').trigger('click')
    await wrapper.vm.$nextTick()

    expect((wrapper.find('.btn-primary').element as HTMLButtonElement).disabled).toBe(true)
  })

  it('poll 无新版本时显示「已经是最新版本」', async () => {
    vi.mocked(triggerPoll).mockResolvedValue({ new_releases: [] })
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.already_latest'))
  })

  it('poll 失败时显示「检查失败」Toast 且按钮恢复', async () => {
    vi.mocked(triggerPoll).mockRejectedValue(new Error('poll err'))
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.check_failed', 'poll err'))
    expect((wrapper.find('.btn-primary').element as HTMLButtonElement).disabled).toBe(false)
  })

  it('sourceChecking 时按钮 disabled 且不触发 triggerPoll', async () => {
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.stub-check-busy').trigger('click')
    await wrapper.vm.$nextTick()

    expect((wrapper.find('.btn-primary').element as HTMLButtonElement).disabled).toBe(true)
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(triggerPoll).not.toHaveBeenCalled()
  })
})

describe('App.vue — 版本统计 computed', () => {
  it('unreadReleaseCounts 按 repo 统计未读（owner/repo 统一小写）', async () => {
    vi.mocked(getReleases).mockResolvedValue([
      { id: 1, source_type: 'github', owner: 'Tauri-Apps', repo: 'Tauri', notification_status: 'pending', snooze_until: null } as never,
      { id: 2, source_type: 'github', owner: 'Tauri-Apps', repo: 'Tauri', notification_status: 'pending', snooze_until: null } as never,
      { id: 3, source_type: 'github', owner: 'vuejs', repo: 'core', notification_status: 'clicked', snooze_until: null } as never,
    ])

    const wrapper = await mountRealApp()
    await flushPromises()

    const counts = wrapper.findComponent(SourceTabStub).props('unreadReleaseCounts') as Record<string, number>
    expect(counts['github|tauri-apps|tauri']).toBe(2)
    expect(counts['github|vuejs|core']).toBeUndefined()
  })

  it('totalReleaseCounts 按 repo 统计总数', async () => {
    vi.mocked(getReleases).mockResolvedValue([
      { id: 1, source_type: 'github', owner: 'a', repo: 'b', notification_status: 'pending' } as never,
      { id: 2, source_type: 'github', owner: 'a', repo: 'b', notification_status: 'clicked' } as never,
      { id: 3, source_type: 'github', owner: 'c', repo: 'd', notification_status: 'ignored' } as never,
    ])

    const wrapper = await mountRealApp()
    await flushPromises()

    const counts = wrapper.findComponent(SourceTabStub).props('totalReleaseCounts') as Record<string, number>
    expect(counts['github|a|b']).toBe(2)
    expect(counts['github|c|d']).toBe(1)
  })
})

describe('App.vue — 跨 Tab 导航（子组件事件入口）', () => {
  it('open-releases 事件跳转到 releases 并设搜索词', async () => {
    const wrapper = await mountRealApp()

    await wrapper.find('.stub-open-releases').trigger('click')

    expect(wrapper.findAll('nav.tabs button')[1].classes()).toContain('active')
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('tauri-apps/tauri')
    expect(wrapper.findComponent(ReleaseTabStub).props('statusFilter')).toBe('all')
  })

  it('open-unread-releases 事件跳转并设未读筛选', async () => {
    const wrapper = await mountRealApp()

    await wrapper.find('.stub-open-unread').trigger('click')

    expect(wrapper.findAll('nav.tabs button')[1].classes()).toContain('active')
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('vuejs/core')
    expect(wrapper.findComponent(ReleaseTabStub).props('statusFilter')).toBe('unread')
  })
})

describe('App.vue — 卸载清理', () => {
  it('卸载时调用 unlisten 清理', async () => {
    const wrapper = await mountRealApp()

    wrapper.unmount()

    expect(mockUnlisten).toHaveBeenCalled()
  })

  it('卸载时清理 countdown 定时器（fake timers）', async () => {
    vi.useFakeTimers()
    const clearIntervalSpy = vi.spyOn(globalThis, 'clearInterval')
    const wrapper = await mountRealApp()

    wrapper.unmount()

    expect(clearIntervalSpy).toHaveBeenCalled()
    clearIntervalSpy.mockRestore()
  })

  it('卸载时清理 toast 定时器', async () => {
    vi.useFakeTimers()
    const clearTimeoutSpy = vi.spyOn(globalThis, 'clearTimeout')
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()

    // 触发一个 toast，创建 setTimeout
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    wrapper.unmount()

    expect(clearTimeoutSpy).toHaveBeenCalled()
    clearTimeoutSpy.mockRestore()
  })

  it('卸载时清理 systemThemeMedia.onchange', async () => {
    const onchangeSpy = vi.fn()
    window.matchMedia = vi.fn().mockReturnValue({ matches: false, onchange: onchangeSpy })
    const wrapper = await mountRealApp()

    wrapper.unmount()

    expect(window.matchMedia).toHaveBeenCalled()
    expect((window.matchMedia as ReturnType<typeof vi.fn>).mock.results[0].value.onchange).toBeNull()
  })
})

describe('App.vue — 异步错误处理', () => {
  it('loadSources 失败时显示 toast 且不产生未捕获 rejection', async () => {
    vi.mocked(listSources).mockRejectedValue(new Error('db locked'))
    const wrapper = await mountRealApp()
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.load_failed', 'db locked'))
  })

  it('loadReleases 失败时显示 toast', async () => {
    vi.mocked(getReleases).mockRejectedValue(new Error('network'))
    const wrapper = await mountRealApp()
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.load_failed', 'network'))
  })

  it('loadSettings 失败时显示 toast', async () => {
    vi.mocked(getSettings).mockRejectedValue(new Error('settings io'))
    const wrapper = await mountRealApp()
    await flushPromises()

    expect(wrapper.find('.toast').text()).toBe(t('app.load_failed', 'settings io'))
  })
})

describe('App.vue — Tauri 事件处理', () => {
  it('poll-completed 触发数据刷新与日志刷新', async () => {
    const wrapper = await mountRealApp()

    vi.mocked(listSources).mockClear()
    vi.mocked(getReleases).mockClear()

    const pollCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'poll-completed')
    expect(pollCall).toBeDefined()
    const refreshKeyBefore = wrapper.findComponent(LogTabStub).props('refreshKey')

    await pollCall![1]({ payload: {} })
    await flushPromises()

    expect(listSources).toHaveBeenCalled()
    expect(getReleases).toHaveBeenCalled()
    expect(wrapper.findComponent(LogTabStub).props('refreshKey')).toBe(refreshKeyBefore + 1)
  })

  it('release-state-changed 触发 releases 刷新', async () => {
    await mountRealApp()

    vi.mocked(getReleases).mockClear()

    const stateCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'release-state-changed')
    expect(stateCall).toBeDefined()

    await stateCall![1]({ payload: {} })

    expect(getReleases).toHaveBeenCalled()
  })

  it('source-auto-disabled 显示 Toast 并刷新数据（真实文案）', async () => {
    const wrapper = await mountRealApp()

    vi.mocked(listSources).mockClear()

    const call = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'source-auto-disabled')
    expect(call).toBeDefined()

    await call![1]({ payload: { owner: 'test-org', repo: 'test-repo', failures: 5 } })
    await flushPromises()

    expect(listSources).toHaveBeenCalled()
    expect(wrapper.find('.toast').text()).toBe(t('app.source_auto_disabled', 'test-org', 'test-repo', '5'))
  })

  it('navigate 事件切换 tab（有效 payload）', async () => {
    const wrapper = await mountRealApp()
    const buttons = wrapper.findAll('nav.tabs button')

    const navigateCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'navigate')
    expect(navigateCall).toBeDefined()

    await navigateCall![1]({ payload: 'releases' })
    expect(buttons[1].classes()).toContain('active')

    await navigateCall![1]({ payload: 'settings' })
    expect(buttons[3].classes()).toContain('active')

    await navigateCall![1]({ payload: 'sources' })
    expect(buttons[0].classes()).toContain('active')
  })

  it('navigate 事件无效 payload 时不切换', async () => {
    const wrapper = await mountRealApp()

    const navigateCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'navigate')
    await navigateCall![1]({ payload: 'invalid' })

    expect(wrapper.findAll('nav.tabs button')[0].classes()).toContain('active')
  })
})

describe('App.vue — 倒计时', () => {
  it('syncCountdown 调用 getPollCountdown 并格式化显示', async () => {
    vi.useFakeTimers()
    vi.mocked(getPollCountdown).mockResolvedValue(125)
    const wrapper = await mountRealApp()

    expect(getPollCountdown).toHaveBeenCalled()
    expect(wrapper.find('.countdown-text').text()).toContain(t('app.min_sec', '2', '5'))
  })

  it('倒计时为 0 时显示「即将检查」', async () => {
    vi.useFakeTimers()
    vi.mocked(getPollCountdown).mockResolvedValue(0)
    const wrapper = await mountRealApp()

    expect(wrapper.find('.countdown-text').text()).toContain(t('app.check_soon'))
  })

  it('倒计时为负数时显示「即将检查」', async () => {
    vi.useFakeTimers()
    vi.mocked(getPollCountdown).mockResolvedValue(-1)
    const wrapper = await mountRealApp()

    expect(wrapper.find('.countdown-text').text()).toContain(t('app.check_soon'))
  })
})

describe('App.vue — 主内容滚动状态', () => {
  it('scrollTop > 0 时 main 有 is-scrolled class，滚回顶部后移除', async () => {
    const wrapper = await mountRealApp()
    const mainEl = wrapper.find('.app-main')
    expect(mainEl.classes()).not.toContain('is-scrolled')

    Object.defineProperty(mainEl.element, 'scrollTop', { value: 10, configurable: true })
    await mainEl.trigger('scroll')
    expect(mainEl.classes()).toContain('is-scrolled')

    Object.defineProperty(mainEl.element, 'scrollTop', { value: 0, configurable: true })
    await mainEl.trigger('scroll')
    expect(mainEl.classes()).not.toContain('is-scrolled')
  })
})

describe('App.vue — 右键菜单（document 事件入口）', () => {
  it('选中文本后右键 → 复制菜单，点击复制写入剪贴板', async () => {
    const mockClipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
    Object.assign(navigator, { clipboard: mockClipboard })
    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => '选中的文本' } as unknown as Selection)

    const wrapper = await mountRealApp()

    document.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 50, clientY: 60 }))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.stub-context-menu').exists()).toBe(true)

    await wrapper.find('.stub-menu-item').trigger('click')
    await flushPromises()

    expect(mockClipboard.writeText).toHaveBeenCalledWith('选中的文本')
  })

  it('无选中文本时右键不显示复制菜单', async () => {
    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => '' } as unknown as Selection)
    const wrapper = await mountRealApp()

    document.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }))
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.stub-context-menu').exists()).toBe(false)
  })

  it('输入框内右键 → 剪切/复制/粘贴/全选菜单，点击执行对应动作', async () => {
    const { readText } = await import('@tauri-apps/plugin-clipboard-manager')
    vi.mocked(readText).mockResolvedValue('粘贴的内容')
    const execCommandMock = vi.fn()
    Object.defineProperty(document, 'execCommand', { value: execCommandMock, writable: true, configurable: true })

    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    const wrapper = await mountRealApp()

    // 打开输入框菜单
    inputEl.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 20 }))
    await wrapper.vm.$nextTick()
    const menuItems = wrapper.findAll('.stub-menu-item')
    expect(menuItems.map(i => i.text())).toEqual([
      t('context.cut'), t('context.copy'), t('context.paste'), t('context.select_all'),
    ])

    // 复制
    await menuItems[1].trigger('click')
    expect(execCommandMock).toHaveBeenCalledWith('copy')

    // 再次打开 → 粘贴
    inputEl.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 20 }))
    await wrapper.vm.$nextTick()
    await wrapper.findAll('.stub-menu-item')[2].trigger('click')
    await flushPromises()
    expect(readText).toHaveBeenCalled()
    expect(execCommandMock).toHaveBeenCalledWith('insertText', false, '粘贴的内容')

    // 再次打开 → 全选
    inputEl.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 20 }))
    await wrapper.vm.$nextTick()
    await wrapper.findAll('.stub-menu-item')[3].trigger('click')
    expect(execCommandMock).toHaveBeenCalledWith('selectAll')

    // 再次打开 → 剪切
    inputEl.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 20 }))
    await wrapper.vm.$nextTick()
    await wrapper.findAll('.stub-menu-item')[0].trigger('click')
    expect(execCommandMock).toHaveBeenCalledWith('cut')

    document.body.removeChild(inputEl)
  })

  it('粘贴读取剪贴板失败时静默处理', async () => {
    const { readText } = await import('@tauri-apps/plugin-clipboard-manager')
    vi.mocked(readText).mockRejectedValue(new Error('clipboard denied'))
    Object.defineProperty(document, 'execCommand', { value: vi.fn(), writable: true, configurable: true })

    const inputEl = document.createElement('input')
    document.body.appendChild(inputEl)
    const wrapper = await mountRealApp()

    inputEl.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 20 }))
    await wrapper.vm.$nextTick()
    await wrapper.findAll('.stub-menu-item')[2].trigger('click')
    await flushPromises()

    // 不抛错即通过
    document.body.removeChild(inputEl)
  })
})

describe('App.vue — check-result 事件（源检查回调）', () => {
  it('源检查结果 0 个新版本 → 「已经是最新版本」', async () => {
    const wrapper = await mountRealApp()

    // 事件入口：SourceTab stub 的 check-result 携带 0（通过真实绑定进入 handleSourceCheckResult）
    const stub = wrapper.findComponent(SourceTabStub)
    stub.vm.$emit('check-result', 0)
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.toast').text()).toBe(t('app.already_latest'))
  })

  it('源检查结果有新版本 → 显示数量', async () => {
    const wrapper = await mountRealApp()

    await wrapper.find('.stub-check-result').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.toast').text()).toBe(t('app.new_found', '2'))
  })
})

describe('App.vue — SettingsTab update 回调（事件入口）', () => {
  it('update(pollChanged=true) 时重启倒计时（重新同步）', async () => {
    const wrapper = await mountRealApp()
    vi.mocked(getPollCountdown).mockClear()

    await wrapper.find('.stub-settings-restart').trigger('click')
    await flushPromises()

    expect(getPollCountdown).toHaveBeenCalled()
  })

  it('update(forceReload=true) 时重新加载 sources/releases', async () => {
    const wrapper = await mountRealApp()
    vi.mocked(listSources).mockClear()
    vi.mocked(getReleases).mockClear()

    await wrapper.find('.stub-settings-reload').trigger('click')
    await flushPromises()

    expect(listSources).toHaveBeenCalled()
    expect(getReleases).toHaveBeenCalled()
  })
})

describe('App.vue — Agent 工作区窗口尺寸', () => {
  it('窄窗口（<900+440）展开：窗口加宽、收起缩窄、多次开关循环尺寸稳定', async () => {
    // 高 DPI 环境：窗口缩放 1.25，innerSize 返回物理像素 1600×900 → 逻辑 1280
    // 1280 < 900+440=1340 → 走「加宽窗口」分支
    mockWindow.innerSize.mockResolvedValue({ width: 1600, height: 900 })
    mockWindow.scaleFactor.mockResolvedValue(1.25)
    mockWindow.setSize.mockResolvedValue(undefined)

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')
    const calls = () => mockWindow.setSize.mock.calls.map((c) => [c[0].width, c[0].height])

    await btn.trigger('click') // 1. 展开：宽度 1280+440，高度不变
    await flushPromises()
    expect(calls()).toEqual([[1600 / 1.25 + 440, 900 / 1.25]])

    await btn.trigger('click') // 2. 收起：当前窗口宽 1280 − 面板 440 = 840
    await flushPromises()
    expect(calls()[1]).toEqual([1600 / 1.25 - 440, 900 / 1.25])

    // 3-6. 再开关两次：尺寸参数完全一致，不累积放大（每次都基于当前 innerSize，不会越缩越小）
    const open = 1600 / 1.25 + 440
    const close = 1600 / 1.25 - 440
    await btn.trigger('click')
    await flushPromises()
    await btn.trigger('click')
    await flushPromises()
    await btn.trigger('click')
    await flushPromises()
    await btn.trigger('click')
    await flushPromises()
    expect(calls()[2]).toEqual([open, 900 / 1.25])
    expect(calls()[3]).toEqual([close, 900 / 1.25])
    expect(calls()[4]).toEqual([open, 900 / 1.25])
    expect(calls()[5]).toEqual([close, 900 / 1.25])

    wrapper.unmount()
  })

  it('宽窗口（≥900+440）展开：不加宽窗口（内部弹出），收起不缩窄', async () => {
    // 窗口逻辑宽 1920 ≥ 1340 → 内部弹出，setSize 零调用
    mockWindow.innerSize.mockResolvedValue({ width: 1920, height: 1080 })
    mockWindow.scaleFactor.mockResolvedValue(1)
    mockWindow.setSize.mockResolvedValue(undefined)

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    await btn.trigger('click') // 展开
    await flushPromises()
    expect(mockWindow.setSize).not.toHaveBeenCalled()
    // 面板已打开：分隔线存在
    expect(wrapper.find('.agent-divider').exists()).toBe(true)

    await btn.trigger('click') // 收起
    await flushPromises()
    expect(mockWindow.setSize).not.toHaveBeenCalled()
    // 面板已关闭：分隔线消失
    expect(wrapper.find('.agent-divider').exists()).toBe(false)
    wrapper.unmount()
  })

  it('最大化时展开：不调用 setSize，面板在窗口内弹出', async () => {
    mockWindow.isMaximized.mockResolvedValue(true)
    mockWindow.innerSize.mockResolvedValue({ width: 1920, height: 1080 })
    mockWindow.scaleFactor.mockResolvedValue(1)
    mockWindow.setSize.mockResolvedValue(undefined)

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    await btn.trigger('click')
    await flushPromises()
    expect(mockWindow.setSize).not.toHaveBeenCalled()

    await btn.trigger('click') // 收起：最大化时不缩窄
    await flushPromises()
    expect(mockWindow.setSize).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('最大化时窄窗口展开：面板压缩到窗口内可容纳宽度（主界面保 710）', async () => {
    mockWindow.isMaximized.mockResolvedValue(true)
    // 窗口逻辑宽 1000（<900+440 但 ≥710+280），面板压缩到 1000−710=290
    mockWindow.innerSize.mockResolvedValue({ width: 1000, height: 800 })
    mockWindow.scaleFactor.mockResolvedValue(1)
    mockWindow.setSize.mockResolvedValue(undefined)

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    await btn.trigger('click')
    await flushPromises()
    expect(mockWindow.setSize).not.toHaveBeenCalled()
    const ws = wrapper.findComponent({ name: 'AgentWorkspace' })
    expect(ws.props('width')).toBe(290)
    wrapper.unmount()
  })

  it('高 DPI 下 window.devicePixelRatio 与 scaleFactor 不一致也不会放大高度', async () => {
    // 模拟 DPR=1（window.devicePixelRatio 失真）但窗口实际缩放 1.5 的极端环境
    mockWindow.innerSize.mockResolvedValue({ width: 1200, height: 800 })
    mockWindow.scaleFactor.mockResolvedValue(1.5)
    mockWindow.setSize.mockResolvedValue(undefined)

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')
    const calls = () => mockWindow.setSize.mock.calls.map((c) => [c[0].width, c[0].height])

    await btn.trigger('click') // 800 逻辑 < 1340 → 加宽
    await flushPromises()
    // 展开：物理 1200/1.5=800 逻辑 +440，高度 800/1.5≈533.33 逻辑（Tauri 内部再乘 1.5 还原物理 800）
    expect(calls()[0][1]).toBeCloseTo(800 / 1.5)
    await btn.trigger('click')
    await flushPromises()
    // 收起：恢复原始逻辑尺寸，高度仍是 533.33（物理 800），不放大
    expect(calls()[1][1]).toBeCloseTo(800 / 1.5)
    wrapper.unmount()
  })

  it('窄窗口打开时完整恢复保存的面板宽度，不被当前窗口宽压缩（回归）', async () => {
    // 应用默认启动窗口宽 750 逻辑 px，DB 中已保存面板宽 700
    mockWindow.innerSize.mockResolvedValue({ width: 750, height: 750 })
    mockWindow.scaleFactor.mockResolvedValue(1)
    mockWindow.setSize.mockResolvedValue(undefined)
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === 'get_agent_ws_width' ? 700 : undefined,
    )
    try {
      const wrapper = await mountRealApp()
      await wrapper.find('.release-agent-btn').trigger('click')
      await flushPromises()

      // 面板恢复保存值 700（而非被当前窗口宽 750 压缩到 280），窗口加宽到 750+700
      const openCall = mockWindow.setSize.mock.calls[0][0] as { width: number; height: number }
      expect(openCall.width).toBe(750 + 700)
      // AgentWorkspace 收到的宽度 prop 为保存值
      const ws = wrapper.findComponent({ name: 'AgentWorkspace' })
      expect(ws.props('width')).toBe(700)
      wrapper.unmount()
    } finally {
      vi.mocked(invoke).mockImplementation(async () => undefined)
    }
  })

  it('真实 Tauri 语义下（setSize 作用于 inner），多次开关循环后窗口 outer 尺寸稳定', async () => {
    // 模拟 Tauri 真实语义（tauri-runtime-wry 源码 WindowMessage::SetSize → set_inner_size）：
    // setSize 设置的是 inner（内容区）；outer = inner + 标题栏/边框。
    // 若代码读写口径混用（如读 outer 当 inner 目标），本测试会因 outer 逐次累积而失败。
    const SCALE = 1.25
    const BORDER_X = 8 // 左右边框总宽（物理 px）
    const CHROME_Y = 39 // 标题栏 + 上下边框总高（物理 px）
    let inner = { width: 1592, height: 861 } // 初始 outer 1600×900 对应的 inner（1592/1.25≈1274 < 1340 → 加宽分支）
    const initialOuter = { width: 1600, height: 900 }

    mockWindow.innerSize.mockImplementation(async () => ({ ...inner }))
    mockWindow.scaleFactor.mockResolvedValue(SCALE)
    mockWindow.setSize.mockImplementation(async (size: { width: number; height: number }) => {
      // Logical → Physical，且 setSize 作用于 inner
      inner = { width: Math.round(size.width * SCALE), height: Math.round(size.height * SCALE) }
    })
    const outerOf = () => ({ width: inner.width + BORDER_X, height: inner.height + CHROME_Y })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    for (let i = 0; i < 5; i++) {
      await btn.trigger('click') // 展开
      await flushPromises()
      expect(outerOf().width).toBe(initialOuter.width + 440 * SCALE) // 仅宽度 +440 逻辑
      expect(outerOf().height).toBe(initialOuter.height) // 高度严格不变
      await btn.trigger('click') // 收起
      await flushPromises()
      expect(outerOf().width).toBe(initialOuter.width) // 完全恢复
      expect(outerOf().height).toBe(initialOuter.height)
    }
    wrapper.unmount()
  })
})
