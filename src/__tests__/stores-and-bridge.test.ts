import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { extractErrorMessage } from '../services/feedback'
import { useCatalogStore } from '../stores/catalog'
import { useSettingsStore } from '../stores/settings'
import { useReleaseFilterStore } from '../stores/releaseFilter'
import { useUiStore } from '../stores/ui'
import { useAgentStore } from '../stores/agent'
import {
  mountEventBridge,
  isEventBridgeMounted,
  subscribeTranslateChunk,
} from '../api/event-bridge'
import { sourceRepoKey } from '../api/sources'
import { createRelease, createSource, defaultSettings } from './helpers'

const {
  listSourcesMock,
  getReleaseCatalogMock,
  triggerPollMock,
  getAgentConfigMock,
  getSettingsMock,
  updateSettingsMock,
  setCredentialMock,
  eventListeners,
  unlistenFns,
} = vi.hoisted(() => {
  const listeners = new Map<string, (e: { payload: unknown }) => void>()
  const unlisteners: ReturnType<typeof vi.fn>[] = []
  return {
    listSourcesMock: vi.fn(),
    getReleaseCatalogMock: vi.fn(),
    triggerPollMock: vi.fn(),
    getAgentConfigMock: vi.fn(),
    getSettingsMock: vi.fn(),
    updateSettingsMock: vi.fn(),
    setCredentialMock: vi.fn(),
    eventListeners: listeners,
    unlistenFns: unlisteners,
  }
})

vi.mock('../api/sources', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/sources')>()
  return { ...actual, listSources: listSourcesMock }
})

vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return {
    ...actual,
    getReleaseCatalog: getReleaseCatalogMock,
    triggerPoll: triggerPollMock,
    getPollCountdown: vi.fn().mockResolvedValue(null),
  }
})

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return { ...actual, getAgentConfig: getAgentConfigMock }
})

vi.mock('../api/settings', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/settings')>()
  return {
    ...actual,
    getSettings: getSettingsMock,
    updateSettings: updateSettingsMock,
    setCredential: setCredentialMock,
  }
})

vi.mock('../bindings', () => {
  const makeEvent = (name: string) => ({
    listen: vi.fn(async (cb: (e: { payload: unknown }) => void) => {
      eventListeners.set(name, cb)
      const off = vi.fn(() => {
        if (eventListeners.get(name) === cb) eventListeners.delete(name)
      })
      unlistenFns.push(off)
      return off
    }),
  })
  return {
    events: {
      navigate: makeEvent('navigate'),
      focusRelease: makeEvent('focusRelease'),
      pollCompleted: makeEvent('pollCompleted'),
      releaseStateChanged: makeEvent('releaseStateChanged'),
      logAppended: makeEvent('logAppended'),
      sourceAutoDisabled: makeEvent('sourceAutoDisabled'),
      agentRunFinished: makeEvent('agentRunFinished'),
      agentRpcStream: makeEvent('agentRpcStream'),
      releaseTranslateChunk: makeEvent('releaseTranslateChunk'),
    },
    commands: {
      getNextPollSeconds: vi.fn().mockResolvedValue(null),
    },
  }
})

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  eventListeners.clear()
  unlistenFns.length = 0
  setActivePinia(createPinia())
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockReturnValue({ matches: false, onchange: null }),
  })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('services/feedback', () => {
  it('extractErrorMessage 剥离 Error: 前缀并兼容多种错误值', () => {
    expect(extractErrorMessage(new Error('Error: network down'))).toBe('network down')
    expect(extractErrorMessage('Error: timeout')).toBe('timeout')
    expect(extractErrorMessage('plain error')).toBe('plain error')
    expect(extractErrorMessage(404)).toBe('404')
  })
})

describe('useCatalogStore', () => {
  it('loadSources 请求序号防护：后发先至时丢弃过期响应', async () => {
    let resolveFirst!: (v: ReturnType<typeof createSource>[]) => void
    let resolveSecond!: (v: ReturnType<typeof createSource>[]) => void
    listSourcesMock
      .mockImplementationOnce(() => new Promise((r) => { resolveFirst = r }))
      .mockImplementationOnce(() => new Promise((r) => { resolveSecond = r }))

    const catalog = useCatalogStore()
    const p1 = catalog.loadSources()
    const p2 = catalog.loadSources()

    const fresh = [createSource({ id: 2, repo: 'fresh' })]
    const stale = [createSource({ id: 1, repo: 'stale' })]
    resolveSecond(fresh)
    await p2
    expect(catalog.sources.map((s) => s.repo)).toEqual(['fresh'])

    resolveFirst(stale)
    await p1
    expect(catalog.sources.map((s) => s.repo)).toEqual(['fresh'])
    expect(catalog.sourcesLoading).toBe(false)
  })

  it('patchRelease / patchReleases 就地局部更新并替换数组引用以失效缓存', () => {
    const catalog = useCatalogStore()
    const key = sourceRepoKey('github', 'a', 'b')
    catalog.releases = [
      createRelease({ id: 1, owner: 'a', repo: 'b', notification_status: 'pending' }),
      createRelease({ id: 2, owner: 'a', repo: 'b', notification_status: 'pending' }),
    ]
    const prevRef = catalog.releases
    expect(catalog.unreadReleaseCounts[key]).toBe(2)
    expect(catalog.totalReleaseCounts[key]).toBe(2)

    catalog.patchRelease(1, { notification_status: 'clicked' })
    expect(catalog.releases).not.toBe(prevRef)
    expect(catalog.unreadReleaseCounts[key]).toBe(1)

    catalog.patchReleases([1, 2], { notification_status: 'ignored' })
    expect(catalog.unreadReleaseCounts[key]).toBeUndefined()
  })

  it('handlePoll 覆盖无源 / 已最新 / 发现新版本 / 异常分支', async () => {
    const catalog = useCatalogStore()
    const ui = useUiStore()
    const toastSpy = vi.spyOn(ui, 'showToast')

    await catalog.handlePoll()
    expect(triggerPollMock).not.toHaveBeenCalled()
    expect(toastSpy).toHaveBeenCalledTimes(1)

    catalog.sources = [createSource({ id: 1, enabled: true })]
    triggerPollMock.mockResolvedValueOnce({ new_releases: [] })
    await catalog.handlePoll()
    expect(triggerPollMock).toHaveBeenCalledTimes(1)

    triggerPollMock.mockResolvedValueOnce({ new_releases: [createRelease({ id: 9 })] })
    await catalog.handlePoll()
    expect(toastSpy).toHaveBeenCalledTimes(3)

    triggerPollMock.mockRejectedValueOnce(new Error('Error: poll fail'))
    await catalog.handlePoll()
    expect(toastSpy).toHaveBeenLastCalledWith(expect.stringContaining('poll fail'))
    expect(catalog.polling).toBe(false)
  })
})

describe('useSettingsStore', () => {
  it('isDraftDirty 响应草稿修改，saveSettings 返回 pollChanged 并提交状态', async () => {
    updateSettingsMock.mockResolvedValue(undefined)
    const store = useSettingsStore()
    store.setSettings({ ...defaultSettings, poll_interval_minutes: 15 })
    expect(store.isDraftDirty).toBe(false)

    store.draft.poll_interval_minutes = 30
    expect(store.isDraftDirty).toBe(true)

    const pollChanged = await store.saveSettings({ ...store.draft })
    expect(pollChanged).toBe(true)
    expect(store.settings.poll_interval_minutes).toBe(30)
    expect(store.isDraftDirty).toBe(false)
  })

  it('watchSystemTheme 与 stopWatchSystemTheme 正确绑定/解绑 mediaQuery', () => {
    const media = { matches: true, onchange: null as (() => void) | null }
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: vi.fn().mockReturnValue(media),
    })

    const store = useSettingsStore()
    store.settings.theme = 'system'
    store.watchSystemTheme()
    expect(typeof media.onchange).toBe('function')
    media.onchange?.()
    expect(document.documentElement.dataset.theme).toBe('dark')

    store.stopWatchSystemTheme()
    expect(media.onchange).toBeNull()
  })
})

describe('useReleaseFilterStore', () => {
  it('runWithoutPersist 挂起落盘，普通修改正常写入 localStorage', () => {
    const store = useReleaseFilterStore()
    store.viewMode = 'aggregated'
    expect(JSON.parse(localStorage.getItem('relwatch.release.view')!).viewMode).toBe('aggregated')

    store.runWithoutPersist(() => {
      store.viewMode = 'simple'
      store.flagFilter = 'flagged'
    })
    expect(store.viewMode).toBe('simple')
    expect(JSON.parse(localStorage.getItem('relwatch.release.view')!).viewMode).toBe('aggregated')
  })

  it('openSourceReleases / focusReleaseById / reportFocusNotFound 状态转换正确', () => {
    const store = useReleaseFilterStore()
    const ui = useUiStore()
    const toastSpy = vi.spyOn(ui, 'showToast')

    store.openSourceUnreadReleases('tauri')
    expect(store.search).toBe('tauri')
    expect(store.statusFilter).toBe('unread')
    expect(ui.activeTab).toBe('releases')

    store.focusReleaseById(42)
    expect(store.focusTarget).toBe(42)
    expect(store.focusToken).toBe(1)

    store.reportFocusNotFound()
    expect(store.focusTarget).toBeNull()
    expect(toastSpy).toHaveBeenCalledTimes(1)

    store.toggleExpandedRepo('github:a/b')
    expect(store.expandedRepos.has('github:a/b')).toBe(true)
    store.setExpandedRepo('github:a/b', false)
    expect(store.expandedRepos.has('github:a/b')).toBe(false)
  })
})

describe('mountEventBridge', () => {
  it('一次性注册全部 9 个事件并路由到对应 Store，卸载后全部注销', async () => {
    listSourcesMock.mockResolvedValue([])
    getReleaseCatalogMock.mockResolvedValue([])

    const ui = useUiStore()
    const catalog = useCatalogStore()
    const filter = useReleaseFilterStore()
    const agent = useAgentStore()

    const toastSpy = vi.spyOn(ui, 'showToast')
    const refreshLogsSpy = vi.spyOn(ui, 'refreshLogs')
    const scheduleLogsSpy = vi.spyOn(ui, 'scheduleRefreshLogs')
    const scheduleReleasesSpy = vi.spyOn(catalog, 'scheduleLoadReleases')
    const onRunFinishedSpy = vi.spyOn(agent, 'onRunFinished').mockResolvedValue(undefined)
    const handleRpcStreamSpy = vi.spyOn(agent, 'handleRpcStream').mockImplementation(() => {})

    const unmount = await mountEventBridge()
    expect(isEventBridgeMounted()).toBe(true)
    expect(eventListeners.size).toBe(9)

    eventListeners.get('navigate')?.({ payload: 'logs' })
    expect(ui.activeTab).toBe('logs')

    eventListeners.get('focusRelease')?.({ payload: 77 })
    expect(filter.focusTarget).toBe(77)

    eventListeners.get('pollCompleted')?.({ payload: null })
    expect(listSourcesMock).toHaveBeenCalled()
    expect(getReleaseCatalogMock).toHaveBeenCalled()
    expect(refreshLogsSpy).toHaveBeenCalled()

    eventListeners.get('releaseStateChanged')?.({ payload: null })
    expect(scheduleReleasesSpy).toHaveBeenCalled()
    expect(scheduleLogsSpy).toHaveBeenCalled()

    eventListeners.get('logAppended')?.({ payload: null })
    expect(scheduleLogsSpy).toHaveBeenCalledTimes(2)

    eventListeners.get('sourceAutoDisabled')?.({
      payload: { owner: 'org', repo: 'app', failures: 5 },
    })
    expect(toastSpy).toHaveBeenCalledWith(expect.stringContaining('org/app'))

    // run_id 一并转发：终态事件需按 run 归属判断是否已过期
    eventListeners.get('agentRunFinished')?.({ payload: { session_key: 's1', run_id: 7 } })
    expect(onRunFinishedSpy).toHaveBeenCalledWith('s1', 7)

    eventListeners.get('agentRpcStream')?.({ payload: { session_key: 's1', raw: '{}' } })
    expect(handleRpcStreamSpy).toHaveBeenCalledWith({ session_key: 's1', raw: '{}' })

    const chunkSpy = vi.fn()
    const unsubChunk = await subscribeTranslateChunk(chunkSpy)
    eventListeners.get('releaseTranslateChunk')?.({ payload: { release_id: 3, delta: '你好' } })
    expect(chunkSpy).toHaveBeenCalledWith({ release_id: 3, delta: '你好' })
    unsubChunk()

    unmount()
    expect(isEventBridgeMounted()).toBe(false)
    expect(unlistenFns).toHaveLength(9)
    for (const off of unlistenFns) {
      expect(off).toHaveBeenCalledTimes(1)
    }

    // 重入保护：重复挂载会先注销前一个实例，且卸载清空 translateChunkSubscribers
    const unmount1 = await mountEventBridge()
    const subSpy = vi.fn()
    await subscribeTranslateChunk(subSpy)
    const unmount2 = await mountEventBridge()
    eventListeners.get('releaseTranslateChunk')?.({ payload: { release_id: 9, delta: 'old-sub-cleared' } })
    expect(subSpy).not.toHaveBeenCalled()
    unmount1()
    expect(isEventBridgeMounted()).toBe(true)
    unmount2()
    expect(isEventBridgeMounted()).toBe(false)
  })

  it('挂载中途任一 listen 抛错时自动回滚已注册监听器', async () => {
    const { events } = await import('../bindings')
    vi.mocked(events.pollCompleted.listen).mockRejectedValueOnce(new Error('ipc fail'))

    await expect(mountEventBridge()).rejects.toThrow('ipc fail')
    expect(isEventBridgeMounted()).toBe(false)
    expect(eventListeners.size).toBe(0)
    for (const off of unlistenFns) {
      expect(off).toHaveBeenCalledTimes(1)
    }
  })
})
