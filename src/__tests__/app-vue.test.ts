import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import fs from 'node:fs'
import path from 'node:path'
import { t, setLocale } from '../i18n'
import { defaultSettings } from './helpers'

// ========== Tauri 边界 Mocks（应用内模块一律走真实实现） ==========
const { mockUnlisten, mockListen, mockWindow, mockCurrentMonitor } = vi.hoisted(() => {
  const mockUnlisten = vi.fn()
  const mockListen = vi.fn((_event: string, _handler: (...args: unknown[]) => void) => Promise.resolve(mockUnlisten))
  const mockWindow = {
    innerSize: vi.fn(),
    scaleFactor: vi.fn(),
    setSize: vi.fn(),
    setMinSize: vi.fn().mockResolvedValue(undefined),
    isMaximized: vi.fn().mockResolvedValue(false),
    outerPosition: vi.fn().mockResolvedValue({ x: 0, y: 0 }),
    outerSize: vi.fn().mockResolvedValue({ width: 0, height: 0 }),
    onResized: vi.fn().mockResolvedValue(mockUnlisten),
  }
  // null = 拿不到显示器信息：面板展开时不做屏幕空间截断
  const mockCurrentMonitor = vi.fn().mockResolvedValue(null)
  return { mockUnlisten, mockListen, mockWindow, mockCurrentMonitor }
})
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => mockWindow,
  currentMonitor: mockCurrentMonitor,
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
      max_concurrency: 3,
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
  return { ...actual, getReleaseCatalog: vi.fn(), getPollCountdown: vi.fn(), triggerPoll: vi.fn() }
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
import { getReleaseCatalog, getPollCountdown, triggerPoll } from '../api/releases'
import { getSettings } from '../api/settings'
import { getAgentConfig } from '../api/agent'
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
  props: ['search', 'statusFilter', 'focusTarget', 'focusToken'],
  emits: ['update', 'focus-consumed', 'focus-not-found'],
  template:
    '<div class="stub-releasetab"><span class="stub-search">{{ search }}</span><span class="stub-status">{{ statusFilter }}</span><span class="stub-focus-target">{{ focusTarget }}</span><span class="stub-focus-token">{{ focusToken }}</span></div>',
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
  emits: ['update', 'agent-config-changed'],
  template: `
    <div class="stub-settingstab">
      <button class="stub-settings-restart" @click="$emit('update', true, false)">restart-countdown</button>
      <button class="stub-settings-reload" @click="$emit('update', false, true)">reload-all</button>
      <button class="stub-agent-config-changed" @click="$emit('agent-config-changed')">agent-config-changed</button>
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

// jsdom 的 innerWidth 固定 1024，与用例设定的窗口逻辑宽不一致会算出错位的列宽
function setInnerWidth(w: number) {
  Object.defineProperty(window, 'innerWidth', { value: w, configurable: true, writable: true })
}

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
  vi.mocked(getReleaseCatalog).mockResolvedValue([])
  vi.mocked(getPollCountdown).mockResolvedValue(300)
  vi.mocked(getSettings).mockResolvedValue(defaultSettings)
  vi.mocked(triggerPoll).mockResolvedValue({ new_releases: [] })
  // 真实 invoke 恒返回 Promise；默认实现也必须是 async，否则调用方 .catch 会炸
  vi.mocked(invoke).mockImplementation(async () => undefined)

  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockReturnValue({ matches: false, onchange: null }),
  })
  // 默认：非最大化（最大化测试会覆盖，beforeEach 复位防残留串扰）
  mockWindow.isMaximized.mockResolvedValue(false)
  // 下面几个 mock 有用例自定义过实现，复位防止外溢到后续用例
  mockWindow.setMinSize.mockImplementation(async () => undefined)
  mockWindow.outerPosition.mockResolvedValue({ x: 0, y: 0 })
  mockWindow.outerSize.mockResolvedValue({ width: 0, height: 0 })
  mockCurrentMonitor.mockResolvedValue(null)
  document.documentElement.dataset.theme = ''
  setLocale('zh-CN')
})

afterEach(() => {
  vi.useRealTimers()
  // 用例可能改写 innerWidth（壳布局按视口宽算列宽），复位避免串扰
  Object.defineProperty(window, 'innerWidth', { value: 1024, configurable: true, writable: true })
  document.documentElement.dataset.theme = ''
  setLocale('zh-CN')
})

describe('App.vue — 挂载初始化', () => {
  it('onMounted 时同时加载 sources / releases / settings', async () => {
    await mountRealApp()

    expect(listSources).toHaveBeenCalled()
    expect(getReleaseCatalog).toHaveBeenCalled()
    expect(getSettings).toHaveBeenCalled()
  })

  it('某个 API 失败时不影响其他 API（Promise.allSettled）', async () => {
    vi.mocked(listSources).mockRejectedValue(new Error('网络错误'))
    vi.mocked(getReleaseCatalog).mockResolvedValue([{ id: 1, tag_name: 'v1' } as never])

    await mountRealApp()

    expect(getReleaseCatalog).toHaveBeenCalled()
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

  it('注册 5 个 Tauri 事件监听器', async () => {
    await mountRealApp()

    expect(mockListen).toHaveBeenCalledWith('navigate', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('poll-completed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('release-state-changed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('source-auto-disabled', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('focus-release', expect.any(Function))
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

    // 两次挂载共 12 次 listen（6 个事件 × 2），全部 unlisten 都被调用（无泄漏）。
    // 事件清单：release-state-changed / poll-completed / log-appended /
    // source-auto-disabled / navigate / focus-release（+ agent 相关另计）。
    // 新增事件时同步更新此数字——它正是「监听器是否泄漏」的哨兵。
    expect(mockListen).toHaveBeenCalledTimes(12)
    expect(mockUnlisten).toHaveBeenCalledTimes(12)
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

  it('短文案 Toast 按下限时长（2.6s）自动隐藏', async () => {
    vi.useFakeTimers()
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(wrapper.find('.toast').exists()).toBe(true)

    vi.advanceTimersByTime(2599)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(true)

    vi.advanceTimersByTime(1)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(false)
  })

  it('长文案 Toast 按字数延长停留', async () => {
    vi.useFakeTimers()
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: true } as never,
    ])
    vi.mocked(triggerPoll).mockRejectedValue(new Error('网络连接超时，请检查代理设置后重试一次'))
    const wrapper = await mountRealApp()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(wrapper.find('.toast').exists()).toBe(true)

    // 短文案此刻早已消失（下限 2.6s），长文案仍在
    vi.advanceTimersByTime(3000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(true)

    // 累计 6s 达到封顶时长后消失
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

    // 悬浮暂停：用户移过来选中复制错误详情/导出路径时不该消失
    await wrapper.find('.toast').trigger('mouseenter')
    vi.advanceTimersByTime(10000)
    expect(wrapper.find('.toast').exists()).toBe(true)

    // 移开后重新计时（本条文案按下限 2.6s）
    await wrapper.find('.toast').trigger('mouseleave')
    vi.advanceTimersByTime(2000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(true)
    vi.advanceTimersByTime(1000)
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.toast').exists()).toBe(false)
  })

  it('Agent 面板打开时 Toast 左移到面板左侧，不压住输入区按钮', async () => {
    vi.useFakeTimers()
    setInnerWidth(1920)
    mockWindow.innerSize.mockResolvedValue({ width: 1920, height: 1080 })
    mockWindow.scaleFactor.mockResolvedValue(1)
    mockWindow.setSize.mockResolvedValue(undefined)
    vi.mocked(listSources).mockResolvedValue([
      { id: 1, owner: 'a', repo: 'b', enabled: false } as never,
    ])
    const wrapper = await mountRealApp()
    const toastRight = () => (wrapper.find('.toast').element as HTMLElement).style.right

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    // 面板关闭：常规右下角 20px
    expect(toastRight()).toBe('20px')

    // 展开面板：右移「面板宽 440 + 分隔线 5 + 边距 20」
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()
    expect(wrapper.find('.agent-divider').exists()).toBe(true)
    expect(toastRight()).toBe('465px')

    // 收起面板：回到常规右下角
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()
    expect(toastRight()).toBe('20px')
  })

  it('Toast 不设 pointer-events: none，文本可被选中复制', () => {
    // 纯 CSS 行为在 jsdom 下读不到计算样式，直接对 SFC 源码做回归守卫。
    // 错误详情、导出/备份路径依赖选中复制，一旦加回 none 就会静默失效。
    const src = fs.readFileSync(path.resolve(process.cwd(), 'src/App.vue'), 'utf-8')
    const start = src.indexOf('.toast {')
    const rule = src.slice(start, src.indexOf('.toast-enter-active', start))
    expect(start).toBeGreaterThan(-1)
    // 注释里会解释「为什么不用 pointer-events」，断言前先剥掉注释
    const declarations = rule.replace(/\/\*[\s\S]*?\*\//g, '')
    expect(declarations).not.toContain('pointer-events')
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
    vi.mocked(getReleaseCatalog).mockResolvedValue([
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

  it('muted 源的新版本仍计入未读（静音仅抑制通知/托盘红点，不影响应用内未读）', async () => {
    // 静音源 + 其 pending release：应用内未读计数应保持（静音不改变 release 的未读语义）
    vi.mocked(listSources).mockResolvedValue([
      { id: 42, source_type: 'github', owner: 'Tauri-Apps', repo: 'Tauri', muted: true } as never,
    ])
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 1, source_type: 'github', owner: 'Tauri-Apps', repo: 'Tauri', notification_status: 'pending', snooze_until: null } as never,
    ])

    const wrapper = await mountRealApp()
    await flushPromises()

    const counts = wrapper.findComponent(SourceTabStub).props('unreadReleaseCounts') as Record<string, number>
    expect(counts['github|tauri-apps|tauri']).toBe(1)
  })

  it('totalReleaseCounts 按 repo 统计总数', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
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

  it('目录含 300 条时计数等于全量（不再被 200 条上限截断）', async () => {
    // 300 条同源：计数必须等于全量（不被 200 条上限截断）
    vi.mocked(getReleaseCatalog).mockResolvedValue(
      Array.from({ length: 300 }, (_, i) => ({
        id: i + 1,
        source_type: 'github',
        owner: 'tauri-apps',
        repo: 'tauri',
        notification_status: 'pending',
        snooze_until: null,
      })) as never,
    )

    const wrapper = await mountRealApp()
    await flushPromises()

    const total = wrapper.findComponent(SourceTabStub).props('totalReleaseCounts') as Record<string, number>
    const unread = wrapper.findComponent(SourceTabStub).props('unreadReleaseCounts') as Record<string, number>
    expect(total['github|tauri-apps|tauri']).toBe(300)
    expect(unread['github|tauri-apps|tauri']).toBe(300)
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

describe('App.vue — 点击通知主体定位 release（focus-release 事件）', () => {
  /** 取出 focus-release 的监听回调，按后端 payload（release id）触发一次。 */
  async function emitFocusRelease(wrapper: Awaited<ReturnType<typeof mountRealApp>>, id: number) {
    const call = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'focus-release')
    expect(call).toBeDefined()
    await call![1]({ payload: id })
    await wrapper.vm.$nextTick()
  }

  function activeTabIsReleases(wrapper: Awaited<ReturnType<typeof mountRealApp>>) {
    return wrapper.findAll('nav.tabs button')[1].classes().includes('active')
  }

  it('focus-release 事件 → 切到版本列表并下发 focusTarget（按 id 精确定位）', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 7, source_type: 'github', owner: 'tauri-apps', repo: 'tauri', tag_name: 'v2' } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 7)

    expect(activeTabIsReleases(wrapper)).toBe(true)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(7)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusToken')).toBe(1)
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('')
    expect(wrapper.findComponent(ReleaseTabStub).props('statusFilter')).toBe('all')
  })

  it('同仓库多条 release 时下发 focusTarget（由 ReleaseTab 按 id 唯一定位，不再搜索词回填）', async () => {
    const releases = [
      { id: 7, source_type: 'github', owner: 'tauri-apps', repo: 'tauri', tag_name: 'v2', release_name: 'R2', body: null, source_description: null },
      { id: 8, source_type: 'github', owner: 'tauri-apps', repo: 'tauri', tag_name: 'v1', release_name: 'R1', body: null, source_description: null },
      { id: 9, source_type: 'github', owner: 'vuejs', repo: 'core', tag_name: 'v3', release_name: 'R3', body: null, source_description: null },
    ]
    vi.mocked(getReleaseCatalog).mockResolvedValue(releases as never)
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 7)
    // 搜索词不再被回填，改由 ReleaseTab 直接按目标 id 定位
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('')
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(7)
  })

  it('release id 不存在时仍切到版本列表（定位与否由 ReleaseTab 依据数据判定）', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 1, source_type: 'github', owner: 'a', repo: 'b', tag_name: 'v1' } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 999)

    expect(activeTabIsReleases(wrapper)).toBe(true)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(999)
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('')
  })

  it('视频源（repo 为空）定位：不做频道名搜索词回填，直接下发 focusTarget', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 3, source_type: 'youtube', owner: 'UCabc', repo: '', tag_name: '', source_description: '某频道' } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 3)

    expect(activeTabIsReleases(wrapper)).toBe(true)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(3)
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('')
  })

  it('视频源且无 source_description 时也直接下发 focusTarget', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 4, source_type: 'bilibili', owner: '12345', repo: '', tag_name: '', source_description: null } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 4)

    expect(activeTabIsReleases(wrapper)).toBe(true)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(4)
    expect(wrapper.findComponent(ReleaseTabStub).props('search')).toBe('')
  })

  it('连续两次 focus-release 时 token 递增（ReleaseTab 依 token 消费最新目标）', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 7, source_type: 'github', owner: 'tauri-apps', repo: 'tauri', tag_name: 'v2' } as never,
      { id: 9, source_type: 'github', owner: 'vuejs', repo: 'core', tag_name: 'v3' } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 7)
    const token1 = wrapper.findComponent(ReleaseTabStub).props('focusToken') as number
    await emitFocusRelease(wrapper, 9)
    const token2 = wrapper.findComponent(ReleaseTabStub).props('focusToken') as number

    expect(token2).toBe(token1 + 1)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(9)
  })

  it('focus-consumed 事件到达时清空 focusTarget', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 7, source_type: 'github', owner: 'tauri-apps', repo: 'tauri', tag_name: 'v2' } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 7)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBe(7)

    const tab = wrapper.findComponent(ReleaseTabStub)
    ;(tab.vm as unknown as { $emit: (e: string) => void }).$emit('focus-consumed')
    await wrapper.vm.$nextTick()
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBeNull()
  })

  it('focus-not-found 事件到达时给 Toast 提示并清空目标', async () => {
    vi.mocked(getReleaseCatalog).mockResolvedValue([
      { id: 1, source_type: 'github', owner: 'a', repo: 'b', tag_name: 'v1' } as never,
    ])
    const wrapper = await mountRealApp()

    await emitFocusRelease(wrapper, 999)
    const tab = wrapper.findComponent(ReleaseTabStub)
    ;(tab.vm as unknown as { $emit: (e: string) => void }).$emit('focus-not-found')
    await wrapper.vm.$nextTick()

    expect(activeTabIsReleases(wrapper)).toBe(true)
    expect(wrapper.findComponent(ReleaseTabStub).props('focusTarget')).toBeNull()
    expect(wrapper.find('.toast').exists()).toBe(true)
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
    vi.mocked(getReleaseCatalog).mockRejectedValue(new Error('network'))
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
    vi.mocked(getReleaseCatalog).mockClear()

    const pollCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'poll-completed')
    expect(pollCall).toBeDefined()
    const refreshKeyBefore = wrapper.findComponent(LogTabStub).props('refreshKey')

    await pollCall![1]({ payload: {} })
    await flushPromises()

    expect(listSources).toHaveBeenCalled()
    expect(getReleaseCatalog).toHaveBeenCalled()
    expect(wrapper.findComponent(LogTabStub).props('refreshKey')).toBe(refreshKeyBefore + 1)
  })

  it('release-state-changed 触发 releases 刷新（50ms 合帧窗口内合并）', async () => {
    await mountRealApp()

    vi.mocked(getReleaseCatalog).mockClear()

    const stateCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'release-state-changed')
    expect(stateCall).toBeDefined()

    await stateCall![1]({ payload: {} })
    // 刷新走 50ms 合帧调度器（与组件 emit('update') 双路径合并成一次重拉），
    // flushPromises 只清微任务等不到定时器，须等真实 timer
    await new Promise((resolve) => setTimeout(resolve, 60))
    await flushPromises()

    expect(getReleaseCatalog).toHaveBeenCalledTimes(1)
  })

  it('release-state-changed 与组件 emit(update) 双路径到达时合并成一次重拉', async () => {
    const wrapper = await mountRealApp()

    vi.mocked(getReleaseCatalog).mockClear()

    const stateCall = vi.mocked(mockListen).mock.calls.find(c => c[0] === 'release-state-changed')
    expect(stateCall).toBeDefined()
    // 同一次用户操作（如标记已读）：后端命令成功 → 组件 emit('update') +
    // 后端 emit release-state-changed，两条路径前后到达
    const stub = wrapper.findComponent(ReleaseTabStub)
    stub.vm.$emit('update')
    await stateCall![1]({ payload: {} })
    await new Promise((resolve) => setTimeout(resolve, 60))
    await flushPromises()

    // 双路径合帧去重：50ms 窗口内只重拉一次全量
    expect(getReleaseCatalog).toHaveBeenCalledTimes(1)
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
  it('选中文本后右键 → 复制菜单，点击复制走 Rust 剪贴板命令', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)
    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => '选中的文本' } as unknown as Selection)

    const wrapper = await mountRealApp()

    document.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 50, clientY: 60 }))
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.stub-context-menu').exists()).toBe(true)

    await wrapper.find('.stub-menu-item').trigger('click')
    await flushPromises()

    // 不再走 navigator.clipboard（右键菜单场景不可靠），统一由 Rust 端写入
    expect(invoke).toHaveBeenCalledWith('set_clipboard_text', { text: '选中的文本' })
  })

  it('复制失败时提示 release.copy_failed（不再静默失败）', async () => {
    // 不用 mockRejectedValueOnce：mountRealApp 期间可能有其他 invoke 调用抢先消费掉
    // 队列里的 Once 实现，导致点击复制时反而走了成功路径。持续 reject，用例末尾恢复。
    vi.mocked(invoke).mockRejectedValue(new Error('clipboard busy'))
    vi.spyOn(window, 'getSelection').mockReturnValue({ toString: () => '选中的文本' } as unknown as Selection)

    const wrapper = await mountRealApp()

    document.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 50, clientY: 60 }))
    await wrapper.vm.$nextTick()
    await wrapper.find('.stub-menu-item').trigger('click')
    await flushPromises()

    expect(wrapper.find('.toast').text()).toContain(t('release.copy_failed'))
    // 恢复默认成功实现（afterEach 只清调用记录，不重置实现）
    vi.mocked(invoke).mockResolvedValue(undefined)
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
    vi.mocked(getReleaseCatalog).mockClear()

    await wrapper.find('.stub-settings-reload').trigger('click')
    await flushPromises()

    expect(listSources).toHaveBeenCalled()
    expect(getReleaseCatalog).toHaveBeenCalled()
  })
})

describe('App.vue — Agent 工作区窗口尺寸（L / R / W 三个宽度的账）', () => {
  // 面板展开期间窗口的最小宽度：主界面列宽 + 分隔线 + 面板 280
  const minWidthFor = (mainW: number) => mainW + 5 + 280

  // 动态 innerSize：setSize 后窗口真的变宽，开关循环的数字才自洽。
  // setWidth 用于模拟用户拖窗口边缘（物理 px）。
  function trackWindow(initial: { width: number; height: number }, scale = 1) {
    let inner = { ...initial }
    mockWindow.innerSize.mockImplementation(async () => ({ ...inner }))
    mockWindow.scaleFactor.mockResolvedValue(scale)
    mockWindow.setSize.mockImplementation(async (size: { width: number; height: number }) => {
      inner = { width: Math.round(size.width * scale), height: Math.round(size.height * scale) }
    })
    return Object.assign(() => ({ ...inner }), {
      setWidth(width: number) {
        inner = { ...inner, width: Math.round(width * scale) }
      },
    })
  }

  it('展开加宽（面板 + 分隔线）、收起缩窄，主界面列宽不变，多次开关尺寸稳定', async () => {
    setInnerWidth(1280)
    trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')
    const calls = () => mockWindow.setSize.mock.calls.map((c) => [c[0].width, c[0].height])

    await btn.trigger('click') // 展开：面板 440 + 分隔线 5，主界面仍是 1280
    await flushPromises()
    expect(calls()[0]).toEqual([1280 + 445, 900])
    expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(440)

    await btn.trigger('click') // 收起：缩掉面板与分隔线，回到 1280
    await flushPromises()
    expect(calls()[1]).toEqual([1280, 900])

    for (let i = 0; i < 3; i++) {
      await btn.trigger('click')
      await flushPromises()
      await btn.trigger('click')
      await flushPromises()
    }
    // 不累积放大，也不越缩越小
    const expected: number[][] = []
    for (let i = 0; i < 3; i++) expected.push([1725, 900], [1280, 900])
    expect(calls().slice(2)).toEqual(expected)
    wrapper.unmount()
  })

  it('拖中间分隔线：只改主界面列宽，面板吸收差额，窗口不重设尺寸', async () => {
    setInnerWidth(1280)
    trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()
    mockWindow.setSize.mockClear()

    const ws = () => wrapper.findComponent({ name: 'AgentWorkspace' })
    expect(ws().props('width')).toBe(440)

    await wrapper.find('.agent-divider').trigger('mousedown', { clientX: 900 })
    document.dispatchEvent(new MouseEvent('mousemove', { clientX: 950 })) // 向右拖 50
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(390) // 主界面 1280→1330，面板 440→390

    document.dispatchEvent(new MouseEvent('mouseup'))
    await flushPromises()
    expect(ws().props('width')).toBe(390)
    expect(mockWindow.setSize).not.toHaveBeenCalled()
    // 主界面变宽 → 展开期间的最小窗宽跟着抬高（面板 280 的保底跟着走）
    expect(mockWindow.setMinSize).toHaveBeenLastCalledWith(
      expect.objectContaining({ width: minWidthFor(1330) }),
    )

    // 松开后的 mousemove 不再生效
    document.dispatchEvent(new MouseEvent('mousemove', { clientX: 1200 }))
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(390)
    wrapper.unmount()
  })

  it('挂载时把窗口最小宽度拉回 710（前端重载后的残留自愈）', async () => {
    setInnerWidth(1280)
    trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    expect(mockWindow.setMinSize).toHaveBeenCalledWith(
      expect.objectContaining({ width: 710, height: 450 }),
    )
    wrapper.unmount()
  })

  it('面板展开期间抬高窗口最小宽度，收起后复位', async () => {
    setInnerWidth(1280)
    trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    await btn.trigger('click')
    await flushPromises()
    expect(mockWindow.setMinSize).toHaveBeenCalledWith(
      expect.objectContaining({ width: minWidthFor(1280), height: 450 }),
    )

    await btn.trigger('click')
    await flushPromises()
    expect(mockWindow.setMinSize).toHaveBeenLastCalledWith(
      expect.objectContaining({ width: 710, height: 450 }),
    )
    wrapper.unmount()
  })

  it('面板打开期间禁用 Agent：自动收回面板并复位窗口最小宽度', async () => {
    setInnerWidth(1280)
    trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()
    expect(wrapper.find('.agent-divider').exists()).toBe(true)
    expect(mockWindow.setMinSize).toHaveBeenLastCalledWith(
      expect.objectContaining({ width: minWidthFor(1280) }),
    )

    // 设置页关掉 Agent 总开关 → loadAgentConfig 读到 enabled=false
    vi.mocked(getAgentConfig).mockResolvedValueOnce({
      enabled: false,
      agent_type: 'pi',
      binary: null,
      model: null,
      working_dir: null,
      prompt_suffix: null,
      timeout_seconds: 300,
      skills: [],
      max_concurrency: 3,
    })
    await wrapper.find('.stub-agent-config-changed').trigger('click')
    await flushPromises()

    expect(wrapper.find('.agent-divider').exists()).toBe(false)
    expect(mockWindow.setMinSize).toHaveBeenLastCalledWith(
      expect.objectContaining({ width: 710, height: 450 }),
    )
    wrapper.unmount()
  })

  it('宽窗口展开：同样只加宽窗口，不挤压主界面', async () => {
    setInnerWidth(1920)
    trackWindow({ width: 1920, height: 1080 })
    mockCurrentMonitor.mockResolvedValue({
      position: { x: 0, y: 0 },
      size: { width: 3840, height: 2160 },
      scaleFactor: 1,
    })
    mockWindow.outerPosition.mockResolvedValue({ x: 0, y: 0 })
    mockWindow.outerSize.mockResolvedValue({ width: 1920, height: 1080 })

    const wrapper = await mountRealApp()
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()

    expect(mockWindow.setSize).toHaveBeenCalledTimes(1)
    expect(mockWindow.setSize.mock.calls[0][0]).toMatchObject({ width: 1920 + 445, height: 1080 })
    expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(440)
    wrapper.unmount()
  })

  it('窗口贴着显示器右边缘：加宽量被截断，缺口由主界面让位；关闭后窗口回到打开前', async () => {
    setInnerWidth(1920)
    trackWindow({ width: 1920, height: 1080 })
    mockCurrentMonitor.mockResolvedValue({
      position: { x: 0, y: 0 },
      size: { width: 2000, height: 1080 },
      scaleFactor: 1,
    })
    mockWindow.outerPosition.mockResolvedValue({ x: 0, y: 0 })
    mockWindow.outerSize.mockResolvedValue({ width: 1920, height: 1080 })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    await btn.trigger('click') // 展开：只有 80px 可用空间
    await flushPromises()
    // 窗口加宽到屏幕宽；面板仍按保存值 440，缺口 365 从主界面让出
    expect(mockWindow.setSize.mock.calls[0][0]).toMatchObject({ width: 2000, height: 1080 })
    expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(440)

    await btn.trigger('click') // 收起
    await flushPromises()
    // 收起后窗宽 = 主界面列宽 1555（打开时让位后的值），不再是打开前的 1920
    expect(mockWindow.setSize.mock.calls[1][0]).toMatchObject({ width: 1555, height: 1080 })
    wrapper.unmount()
  })

  it('拖窗口边缘把面板拉宽后再关闭：主界面不跟着变宽（回归）', async () => {
    setInnerWidth(1280)
    const read = trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')
    const ws = () => wrapper.findComponent({ name: 'AgentWorkspace' })

    await btn.trigger('click') // 展开：1280 → 1725（面板 440）
    await flushPromises()
    expect(read().width).toBe(1725)
    expect(ws().props('width')).toBe(440)

    // 用户拖窗口右边缘把面板拉宽 300：主界面不动，面板吃满
    read.setWidth(2025)
    setInnerWidth(2025)
    window.dispatchEvent(new Event('resize'))
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(740)

    await btn.trigger('click') // 关闭
    await flushPromises()
    // 回到打开前的窗宽：面板那 300px 不会落到主界面上
    expect(mockWindow.setSize.mock.calls.at(-1)![0]).toMatchObject({ width: 1280, height: 900 })
    expect(read().width).toBe(1280)
    wrapper.unmount()
  })

  it('展开 → 调宽窗口 → 拖分隔条 → 收起 → 再展开：L 与 R 都被记住', async () => {
    // 用假 DB 承接面板宽的读写，验证「上次关闭时的 R」被原样恢复
    let savedPanelW = 0
    vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'get_agent_ws_width') return savedPanelW
      if (cmd === 'save_agent_ws_width') {
        savedPanelW = (args as { width: number }).width
        return undefined
      }
      return undefined
    })
    setInnerWidth(1280)
    const read = trackWindow({ width: 1280, height: 900 })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')
    const ws = () => wrapper.findComponent({ name: 'AgentWorkspace' })
    const lastSize = () => mockWindow.setSize.mock.calls.at(-1)![0] as { width: number; height: number }

    await btn.trigger('click') // 1. 展开：L 1280 + 分隔线 + R 440
    await flushPromises()
    expect(read().width).toBe(1725)
    expect(ws().props('width')).toBe(440)

    read.setWidth(2025) // 2. 整体调宽 300：只加到面板上
    setInnerWidth(2025)
    window.dispatchEvent(new Event('resize'))
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(740)

    // 3. 拖分隔条右移 100：L 1280→1380、R 740→640，两边都变
    await wrapper.find('.agent-divider').trigger('mousedown', { clientX: 1300 })
    document.dispatchEvent(new MouseEvent('mousemove', { clientX: 1400 }))
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(640)
    document.dispatchEvent(new MouseEvent('mouseup'))
    await flushPromises()

    // 4. 收起：窗口变成主界面列宽 1380（不是打开前的 1280），R 640 落库
    await btn.trigger('click')
    await flushPromises()
    expect(lastSize()).toMatchObject({ width: 1380, height: 900 })
    expect(read().width).toBe(1380)
    expect(savedPanelW).toBe(640)

    // 5. 收起期间调窗口宽度只改 L，不影响下一次的 R
    read.setWidth(1000)
    setInnerWidth(1000)
    window.dispatchEvent(new Event('resize'))
    await wrapper.vm.$nextTick()

    // 6. 再展开：L 跟随当前窗宽 1000，R 恢复上次关闭时的 640
    await btn.trigger('click')
    await flushPromises()
    expect(lastSize()).toMatchObject({ width: 1000 + 645, height: 900 })
    expect(ws().props('width')).toBe(640)
    wrapper.unmount()
  })

  it('展开途中到达的 resize 事件不会把 L 反写成整窗宽（竞态回归）', async () => {
    setInnerWidth(1280)
    trackWindow({ width: 1280, height: 900 })
    // setSize 已生效、resize 事件在 open() 还没落位时到达
    mockWindow.setMinSize.mockImplementation(async (size: { width: number }) => {
      if (size.width === minWidthFor(1280)) {
        setInnerWidth(1725)
        window.dispatchEvent(new Event('resize'))
      }
    })

    const wrapper = await mountRealApp()
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()

    // L 仍是 1280，面板 440（若被反写成 1725，面板会被压到 280）
    expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(440)
    wrapper.unmount()
  })

  it('最大化时展开：不调用 setSize，保存的面板宽度仍生效（主界面让位）', async () => {
    mockWindow.isMaximized.mockResolvedValue(true)
    setInnerWidth(1000)
    trackWindow({ width: 1000, height: 800 })
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === 'get_agent_ws_width' ? 700 : undefined,
    )
    try {
      const wrapper = await mountRealApp()
      const btn = wrapper.find('.release-agent-btn')

      await btn.trigger('click')
      await flushPromises()
      expect(mockWindow.setSize).not.toHaveBeenCalled()
      // 窗口 1000 装不下 主界面 710 + 面板 700：面板压到剩余宽度，而不是固定 280
      expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(285)

      await btn.trigger('click') // 收起：最大化时不缩窄
      await flushPromises()
      expect(mockWindow.setSize).not.toHaveBeenCalled()
      wrapper.unmount()
    } finally {
      vi.mocked(invoke).mockImplementation(async () => undefined)
    }
  })

  it('最大化在宽屏上展开：保存的面板宽度完整生效', async () => {
    mockWindow.isMaximized.mockResolvedValue(true)
    setInnerWidth(1920)
    trackWindow({ width: 1920, height: 1080 })
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === 'get_agent_ws_width' ? 700 : undefined,
    )
    try {
      const wrapper = await mountRealApp()
      await wrapper.find('.release-agent-btn').trigger('click')
      await flushPromises()
      expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(700)
      wrapper.unmount()
    } finally {
      vi.mocked(invoke).mockImplementation(async () => undefined)
    }
  })

  it('窗口变窄再变宽：主界面列宽回到用户设定，不被永久改写', async () => {
    setInnerWidth(1920)
    trackWindow({ width: 1920, height: 1080 })
    mockCurrentMonitor.mockResolvedValue({
      position: { x: 0, y: 0 },
      size: { width: 3840, height: 2160 },
      scaleFactor: 1,
    })
    mockWindow.outerPosition.mockResolvedValue({ x: 0, y: 0 })
    mockWindow.outerSize.mockResolvedValue({ width: 1920, height: 1080 })

    const wrapper = await mountRealApp()
    await wrapper.find('.release-agent-btn').trigger('click')
    await flushPromises()
    const ws = () => wrapper.findComponent({ name: 'AgentWorkspace' })
    expect(ws().props('width')).toBe(440)

    // 窗口被拖窄到 1000：面板先压到最小 280（主界面由 flex 临时挤压）
    setInnerWidth(1000)
    window.dispatchEvent(new Event('resize'))
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(280)

    // 再拖回原宽：主界面恢复原设定，面板回到 440（而不是把主界面永久压成 710）
    setInnerWidth(1920 + 445)
    window.dispatchEvent(new Event('resize'))
    await wrapper.vm.$nextTick()
    expect(ws().props('width')).toBe(440)
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

    await btn.trigger('click') // 800 逻辑宽 → 加宽
    await flushPromises()
    // 展开：高度 800/1.5≈533.33 逻辑（Tauri 内部再乘 1.5 还原物理 800）
    expect(calls()[0][1]).toBeCloseTo(800 / 1.5)
    await btn.trigger('click')
    await flushPromises()
    // 收起：高度仍是 533.33（物理 800），不放大
    expect(calls()[1][1]).toBeCloseTo(800 / 1.5)
    wrapper.unmount()
  })

  it('窄窗口打开时完整恢复保存的面板宽度，不被当前窗口宽压缩（回归）', async () => {
    // 应用默认启动窗口宽 750 逻辑 px，DB 中已保存面板宽 700
    setInnerWidth(750)
    trackWindow({ width: 750, height: 750 })
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === 'get_agent_ws_width' ? 700 : undefined,
    )
    try {
      const wrapper = await mountRealApp()
      await wrapper.find('.release-agent-btn').trigger('click')
      await flushPromises()

      // 窗口加宽 面板 700 + 分隔线 5（而非被当前窗口宽压缩到 280）
      expect(mockWindow.setSize.mock.calls[0][0]).toMatchObject({ width: 750 + 705, height: 750 })
      expect(wrapper.findComponent({ name: 'AgentWorkspace' }).props('width')).toBe(700)
      wrapper.unmount()
    } finally {
      vi.mocked(invoke).mockImplementation(async () => undefined)
    }
  })

  it('真实 Tauri 语义下（setSize 作用于 inner），多次开关循环后窗口 outer 尺寸稳定', async () => {
    // 模拟 Tauri 真实语义（tauri-runtime-wry 源码 WindowMessage::SetSize → set_inner_size）：
    // setSize 设置的是 inner（内容区）；outer = inner + 标题栏/边框。
    // 若代码读写口径混用（如读 outer 当 inner 目标），本测试会因 outer 逐次累积而失败。
    const SCALE = 2 // 200% 缩放：逻辑宽 1280 → inner 2560，全程整数
    const BORDER_X = 8 // 左右边框总宽（物理 px）
    const CHROME_Y = 39 // 标题栏 + 上下边框总高（物理 px）
    const initialOuter = { width: 2560 + BORDER_X, height: 1722 + CHROME_Y }
    setInnerWidth(1280)
    const read = trackWindow({ width: 2560, height: 1722 }, SCALE)
    const outer = () => ({ width: read().width + BORDER_X, height: read().height + CHROME_Y })

    const wrapper = await mountRealApp()
    const btn = wrapper.find('.release-agent-btn')

    for (let i = 0; i < 5; i++) {
      await btn.trigger('click') // 展开
      await flushPromises()
      expect(outer().width).toBe(initialOuter.width + 445 * SCALE) // 仅宽度 +面板+分隔线
      expect(outer().height).toBe(initialOuter.height) // 高度严格不变
      await btn.trigger('click') // 收起
      await flushPromises()
      expect(outer().width).toBe(initialOuter.width) // 完全恢复
      expect(outer().height).toBe(initialOuter.height)
    }
    wrapper.unmount()
  })
})

// ── 全局错误兜底：App 把 showToast 注册为上报出口，异常必须能被用户看见 ──
describe('App.vue — 全局错误兜底的 toast 出口（V2）', () => {
  it('reportFrontendError 弹出 toast 并把同一明细落库', async () => {
    const wrapper = await mountRealApp()
    const { reportFrontendError, resetReportThrottle } = await import('../api/report-error')
    resetReportThrottle()

    await reportFrontendError('ui.unhandled_rejection', new Error('boom'))
    await flushPromises()
    await wrapper.vm.$nextTick()

    const toast = wrapper.find('.toast')
    expect(toast.exists()).toBe(true)
    expect(toast.text()).toContain('boom')
    // 模板占位符必须已被替换（否则用户看到裸的 {error}）
    expect(toast.text()).not.toContain('{error}')

    expect(invoke).toHaveBeenCalledWith('report_frontend_error', expect.objectContaining({
      messageKey: 'ui.unhandled_rejection',
    }))
    wrapper.unmount()
  })
})
