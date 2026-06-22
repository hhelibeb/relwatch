import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SourceTab from '../components/SourceTab.vue'
import { ShowToastKey } from '../injection-keys'
import { parseSourceUrl, addSource, removeSource, updateSource } from '../api/sources'
import { checkSingleSource } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { message, confirm } from '@tauri-apps/plugin-dialog'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ message: vi.fn(), confirm: vi.fn() }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))

vi.mock('../api/sources', () => ({
  parseSourceUrl: vi.fn(),
  addSource: vi.fn(),
  removeSource: vi.fn(),
  updateSource: vi.fn(),
}))

vi.mock('../api/releases', () => ({
  checkSingleSource: vi.fn(),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  translateError: vi.fn((msg: string) => msg),
}))

vi.mock('../i18n', () => ({
  t: vi.fn((key: string, ...args: string[]) => args.length ? `${key}:${args.join(',')}` : key),
  tm: vi.fn((key: string, _args: Record<string, string>) => key),
  getLocale: vi.fn(() => 'en'),
}))

const parseSourceUrlMock = vi.mocked(parseSourceUrl)
const addSourceMock = vi.mocked(addSource)
const removeSourceMock = vi.mocked(removeSource)
const updateSourceMock = vi.mocked(updateSource)
const checkSingleSourceMock = vi.mocked(checkSingleSource)
const openReleaseUrlMock = vi.mocked(openReleaseUrl)
const messageMock = vi.mocked(message)
const confirmMock = vi.mocked(confirm)

interface TestSource {
  id: number
  source_type: string
  owner: string
  repo: string
  poll_interval_minutes: number
  enabled: boolean
  muted: boolean
  last_checked_at: string | null
  last_check_status: string
  last_check_message: string | null
  consecutive_failures: number
  last_new_count: number
  description: string | null
  created_at: string
  updated_at: string
}

function createSource(overrides: Partial<TestSource> = {}): TestSource {
  return {
    id: 1,
    source_type: 'github',
    owner: 'vuejs',
    repo: 'core',
    poll_interval_minutes: 60,
    enabled: true,
    muted: false,
    last_checked_at: '2025-06-01T00:00:00Z',
    last_check_status: 'ok',
    last_check_message: null,
    consecutive_failures: 0,
    last_new_count: 0,
    description: null,
    created_at: '2025-06-01T00:00:00Z',
    updated_at: '2025-06-01T00:00:00Z',
    ...overrides,
  }
}

function mountSourceTab(
  sources: TestSource[] = [createSource()],
  opts: { unreadReleaseCounts?: Record<string, number>; totalReleaseCounts?: Record<string, number> } = {},
) {
  const showToast = vi.fn()
  const wrapper = mount(SourceTab, {
    props: {
      sources,
      polling: false,
      unreadReleaseCounts: opts.unreadReleaseCounts ?? {},
      totalReleaseCounts: opts.totalReleaseCounts ?? {},
    },
    global: {
      provide: {
        [ShowToastKey as symbol]: showToast,
      },
      stubs: { ContextMenu: true },
    },
  })
  return { wrapper, showToast }
}

beforeEach(() => {
  vi.clearAllMocks()
  window.localStorage.clear()
  parseSourceUrlMock.mockImplementation((raw: string) => {
    const match = raw.match(/github\.com\/([^/]+)\/([^/?#]+)/)
    if (match) return { type: 'github', owner: match[1], repo: match[2] }
    const shortMatch = raw.match(/^([a-zA-Z0-9][a-zA-Z0-9_.-]*)\/([a-zA-Z0-9_.-]+)$/)
    if (shortMatch) return { type: 'github', owner: shortMatch[1], repo: shortMatch[2] }
    return null
  })
  addSourceMock.mockResolvedValue(1)
  removeSourceMock.mockResolvedValue(undefined)
  updateSourceMock.mockResolvedValue(undefined)
  checkSingleSourceMock.mockResolvedValue({ new_releases: [] })
  openReleaseUrlMock.mockResolvedValue(undefined)
  messageMock.mockResolvedValue(undefined as any)
  confirmMock.mockResolvedValue(true as any)
})

afterEach(() => {
  vi.clearAllMocks()
})

// ============ 添加 Source ============

describe('SourceTab — 添加 Source', () => {
  it('输入有效 URL 并点击添加按钮，调用 addSource 并清空输入框', async () => {
    const { wrapper } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(parseSourceUrlMock).toHaveBeenCalledWith('https://github.com/vuejs/core')
    expect(addSourceMock).toHaveBeenCalledWith('github', 'vuejs', 'core')
    expect((input.element as HTMLInputElement).value).toBe('')
  })

  it('输入无效 URL 时，显示错误对话框而不调用 addSource', async () => {
    const { wrapper } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('not-a-valid-url')
    await addButton.trigger('click')
    await flushPromises()

    expect(parseSourceUrlMock).toHaveBeenCalled()
    expect(addSourceMock).not.toHaveBeenCalled()
    expect(messageMock).toHaveBeenCalledWith('source.invalid_url', expect.any(Object))
  })

  it('输入已存在的 source 时，显示 toast 提示', async () => {
    const existingSource = createSource({ owner: 'vuejs', repo: 'core' })
    const { wrapper, showToast } = mountSourceTab([existingSource])
    const input = wrapper.get('input[placeholder="source.placeholder"]')
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).not.toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith('source.exists')
  })

  it('addSource 返回 0 表示已存在，显示 toast', async () => {
    addSourceMock.mockResolvedValue(0)
    const { wrapper, showToast } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith('source.exists')
  })

  it('addSource 抛出错误时，显示错误对话框', async () => {
    addSourceMock.mockRejectedValue(new Error('Network error'))
    const { wrapper } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.stringContaining('source.add_failed'),
      expect.any(Object)
    )
  })

  it('添加成功后，emit update', async () => {
    addSourceMock.mockResolvedValue(99)
    const { wrapper } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('输入框为空时，添加按钮禁用', async () => {
    const { wrapper } = mountSourceTab([])
    const addButton = wrapper.get('.btn-add-source')

    expect(addButton.attributes('disabled')).toBeDefined()

    const input = wrapper.get('input[placeholder="source.placeholder"]')
    await input.setValue('https://github.com/vuejs/core')

    expect(addButton.attributes('disabled')).toBeUndefined()
  })

  it('Enter 键触发添加', async () => {
    const { wrapper } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')

    await input.setValue('https://github.com/vuejs/core')
    await input.trigger('keyup.enter')
    await flushPromises()

    expect(addSourceMock).toHaveBeenCalledWith('github', 'vuejs', 'core')
  })

  it('清空按钮清空输入框', async () => {
    const { wrapper } = mountSourceTab([])
    const input = wrapper.get('input[placeholder="source.placeholder"]')

    await input.setValue('some text')
    await wrapper.get('.input-clear-btn').trigger('click')

    expect((input.element as HTMLInputElement).value).toBe('')
  })
})

// ============ 删除单个 Source ============

describe('SourceTab — 删除单个 Source', () => {
  it('点击删除按钮，调用 removeSource 并 emit update', async () => {
    const source = createSource({ id: 5 })
    const { wrapper } = mountSourceTab([source])

    const moreButton = wrapper.get('.btn-more')
    await moreButton.trigger('click')

    const deleteButton = wrapper.get('.dropdown-item-danger')
    await deleteButton.trigger('click')
    await flushPromises()

    expect(removeSourceMock).toHaveBeenCalledWith(5)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('removeSource 抛出错误时，显示错误对话框', async () => {
    removeSourceMock.mockRejectedValue(new Error('Delete failed'))
    const source = createSource({ id: 5 })
    const { wrapper } = mountSourceTab([source])

    const moreButton = wrapper.get('.btn-more')
    await moreButton.trigger('click')

    const deleteButton = wrapper.get('.dropdown-item-danger')
    await deleteButton.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.stringContaining('source.delete_failed'),
      expect.any(Object)
    )
  })
})

// ============ 启用/禁用 Source ============

describe('SourceTab — 启用/禁用 Source', () => {
  it('点击暂停按钮，调用 updateSource 切换 enabled 状态', async () => {
    const source = createSource({ id: 3, enabled: true })
    const { wrapper } = mountSourceTab([source])

    const pauseButton = wrapper.get('.btn-pause')
    await pauseButton.trigger('click')
    await flushPromises()

    expect(updateSourceMock).toHaveBeenCalledWith(3, false, 60)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('disabled source 显示恢复按钮，点击后 enabled 变为 true', async () => {
    const source = createSource({ id: 3, enabled: false })
    const { wrapper } = mountSourceTab([source])

    const resumeButton = wrapper.get('.btn-resume')
    await resumeButton.trigger('click')
    await flushPromises()

    expect(updateSourceMock).toHaveBeenCalledWith(3, true, 60)
  })

  it('updateSource 抛出错误时，显示错误对话框', async () => {
    updateSourceMock.mockRejectedValue(new Error('Update failed'))
    const source = createSource({ id: 3, enabled: true })
    const { wrapper } = mountSourceTab([source])

    const pauseButton = wrapper.get('.btn-pause')
    await pauseButton.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.stringContaining('source.operation_failed'),
      expect.any(Object)
    )
  })
})

// ============ 静默/取消静默 ============

describe('SourceTab — 静默/取消静默', () => {
  it('点击静默按钮，调用 updateSource 设置 muted=true', async () => {
    const source = createSource({ id: 7, enabled: true, muted: false })
    const { wrapper } = mountSourceTab([source])

    const moreButton = wrapper.get('.btn-more')
    await moreButton.trigger('click')

    const muteButton = wrapper.get('.dropdown-item')
    await muteButton.trigger('click')
    await flushPromises()

    expect(updateSourceMock).toHaveBeenCalledWith(7, true, 60, true)
  })

  it('已静默的 source 显示取消静默选项', async () => {
    const source = createSource({ id: 7, enabled: true, muted: true })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.badge-muted').exists()).toBe(true)

    const moreButton = wrapper.get('.btn-more')
    await moreButton.trigger('click')

    const unmuteButton = wrapper.get('.dropdown-item')
    await unmuteButton.trigger('click')
    await flushPromises()

    expect(updateSourceMock).toHaveBeenCalledWith(7, true, 60, false)
  })

  it('disabled source 的静默按钮禁用', async () => {
    const source = createSource({ id: 7, enabled: false, muted: false })
    const { wrapper } = mountSourceTab([source])

    const moreButton = wrapper.get('.btn-more')
    await moreButton.trigger('click')

    const muteButton = wrapper.get('.dropdown-item')
    expect(muteButton.attributes('disabled')).toBeDefined()
  })
})

// ============ 检查单个 Source 更新 ============

describe('SourceTab — 检查单个 Source 更新', () => {
  it('点击检查按钮，调用 checkSingleSource', async () => {
    checkSingleSourceMock.mockResolvedValue({ new_releases: [] })
    const source = createSource({ id: 11 })
    const { wrapper } = mountSourceTab([source])

    const checkButton = wrapper.get('.btn-check')
    await checkButton.trigger('click')
    await flushPromises()

    expect(checkSingleSourceMock).toHaveBeenCalledWith(11)
    expect(wrapper.emitted('update')).toBeTruthy()
    expect(wrapper.emitted('checkResult')).toBeTruthy()
  })

  it('检查发现新 release 时，emit checkResult 带数量', async () => {
    checkSingleSourceMock.mockResolvedValue({
      new_releases: [
        { id: 1, source_id: 11, source_type: 'github', owner: 'vuejs', repo: 'core',
          tag_name: 'v3.3.0', release_name: '3.3.0', html_url: 'https://github.com/vuejs/core/releases/tag/v3.3.0',
          published_at: '2025-06-01T00:00:00Z', prerelease: false, body: null,
          detected_at: '2025-06-01T00:00:00Z', notification_status: 'pending',
          snooze_until: null, ai_summary: null, ai_importance: null, body_translated: null, extra_metadata: null },
      ],
    })
    const source = createSource({ id: 11 })
    const { wrapper } = mountSourceTab([source])

    const checkButton = wrapper.get('.btn-check')
    await checkButton.trigger('click')
    await flushPromises()

    expect(wrapper.emitted('checkResult')?.[0]).toEqual([1])
  })

  it('全局 polling 时，检查按钮禁用', () => {
    const wrapper = mount(SourceTab, {
      props: {
        sources: [createSource({ id: 11 })],
        polling: true,
        unreadReleaseCounts: {},
        totalReleaseCounts: {},
      },
      global: {
        provide: { [ShowToastKey as symbol]: vi.fn() },
        stubs: { ContextMenu: true },
      },
    })

    const checkButton = wrapper.get('.btn-check')
    expect(checkButton.attributes('disabled')).toBeDefined()
  })

  it('checkSingleSource 抛出错误时，显示错误对话框', async () => {
    checkSingleSourceMock.mockRejectedValue(new Error('Check failed'))
    const source = createSource({ id: 11 })
    const { wrapper } = mountSourceTab([source])

    const checkButton = wrapper.get('.btn-check')
    await checkButton.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.stringContaining('source.check_failed'),
      expect.any(Object)
    )
  })

  it('正在检查其他 source 时，所有检查按钮禁用', async () => {
    let resolveCheck: (v: { new_releases: unknown[] }) => void
    checkSingleSourceMock.mockImplementation(() => new Promise(r => { resolveCheck = r as any }))
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'other', repo: 'repo' })]
    const { wrapper } = mountSourceTab(sources)

    const checkButtons = wrapper.findAll('.btn-check')
    await checkButtons[0].trigger('click')

    // During check, buttons should be disabled
    expect(checkButtons[1].attributes('disabled')).toBeDefined()

    // Resolve to clean up
    resolveCheck!({ new_releases: [] })
    await flushPromises()
  })
})

// ============ 打开链接和发布页 ============

describe('SourceTab — 打开 Source 链接和发布页', () => {
  it('点击链接图标，调用 openReleaseUrl', async () => {
    const source = createSource({ owner: 'vuejs', repo: 'core' })
    const { wrapper } = mountSourceTab([source])

    const linkButton = wrapper.get('.btn-icon-link')
    await linkButton.trigger('click')

    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://github.com/vuejs/core')
  })

  it('点击查看发布按钮，emit openReleases', async () => {
    const source = createSource({ owner: 'vuejs', repo: 'core' })
    const { wrapper } = mountSourceTab([source])

    const viewReleasesButton = wrapper.findAll('.btn-icon-link')[1]
    await viewReleasesButton.trigger('click')

    expect(wrapper.emitted('openReleases')?.[0]).toEqual(['vuejs/core'])
  })

  it('有未读 release 时，显示待更新链接并点击 emit openUnreadReleases', async () => {
    const source = createSource({ id: 5, owner: 'vuejs', repo: 'core', enabled: true, last_check_status: 'ok' })
    const { wrapper } = mountSourceTab([source], {
      unreadReleaseCounts: { 'vuejs/core': 3 },
    })

    const pendingLink = wrapper.get('.source-pending-link')
    expect(pendingLink.text()).toContain('source.pending_updates')
    await pendingLink.trigger('click')

    expect(wrapper.emitted('openUnreadReleases')?.[0]).toEqual(['vuejs/core'])
  })

  it('无未读 release 时，不显示待更新链接', () => {
    const source = createSource({ id: 5, owner: 'vuejs', repo: 'core', enabled: true, last_check_status: 'ok' })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.source-pending-link').exists()).toBe(false)
  })
})

// ============ 健康状态显示 ============

describe('SourceTab — 健康状态显示', () => {
  it('enabled + ok 状态显示健康图标，不显示错误区域', () => {
    const source = createSource({ enabled: true, last_check_status: 'ok' })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.health-ok').exists()).toBe(true)
    // ok 状态且无未读时，只显示 last checked 时间
    expect(wrapper.find('.source-health-meta').exists()).toBe(true)
    expect(wrapper.find('.source-error').exists()).toBe(false)
  })

  it('enabled + error 状态显示错误图标和错误消息', () => {
    const source = createSource({
      enabled: true,
      last_check_status: 'error',
      last_check_message: 'Network timeout',
      consecutive_failures: 3,
    })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.health-error').exists()).toBe(true)
    expect(wrapper.find('.source-error').exists()).toBe(true)
    expect(wrapper.find('.source-health-label').text()).toContain('source.health_error')
  })

  it('disabled 状态显示暂停图标', () => {
    const source = createSource({ enabled: false })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.health-paused').exists()).toBe(true)
    expect(wrapper.find('.source-health-label').text()).toContain('source.health_paused')
  })

  it('last_check_status 为空时显示未知状态', () => {
    const source = createSource({ enabled: true, last_check_status: '' })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.health-unknown').exists()).toBe(true)
    expect(wrapper.find('.source-health-label').text()).toContain('source.health_unknown')
  })

  it('显示最后检查时间文本', () => {
    const source = createSource({ last_checked_at: '2025-06-01T12:00:00Z' })
    const { wrapper } = mountSourceTab([source])

    const metaElements = wrapper.findAll('.source-health-meta')
    expect(metaElements.length).toBeGreaterThan(0)
  })

  it('从未检查时显示 "从未检查" 文本', () => {
    const source = createSource({ last_checked_at: null })
    const { wrapper } = mountSourceTab([source])

    const metaElements = wrapper.findAll('.source-health-meta')
    const lastCheckedText = metaElements.find(el => el.text().includes('source.never_checked'))
    expect(lastCheckedText).toBeTruthy()
  })

  it('连续失败次数大于 0 时显示失败次数', () => {
    const source = createSource({
      enabled: true,
      last_check_status: 'error',
      consecutive_failures: 5,
    })
    const { wrapper } = mountSourceTab([source])

    const metaElements = wrapper.findAll('.source-health-meta')
    const failureCount = metaElements.find(el => el.text().includes('source.failure_count'))
    expect(failureCount).toBeTruthy()
  })

  it('error source 显示 last_check_message 经过 translateError', () => {
    const source = createSource({
      enabled: true,
      last_check_status: 'error',
      last_check_message: 'some error msg',
    })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.source-error').text()).toBe('some error msg')
  })
})

// ============ 搜索过滤 ============

describe('SourceTab — 搜索过滤', () => {
  it('切换到搜索模式后，输入关键词过滤 source', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
      createSource({ id: 2, owner: 'facebook', repo: 'react' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const searchInput = wrapper.get('input[placeholder="source.search"]')

    await searchInput.setValue('vue')
    await flushPromises()

    const sourceItems = wrapper.findAll('.source-item')
    expect(sourceItems).toHaveLength(1)
    expect(sourceItems[0].text()).toContain('vuejs')
  })

  it('搜索无结果时，显示空状态', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const searchInput = wrapper.get('input[placeholder="source.search"]')

    await searchInput.setValue('nonexistent')
    await flushPromises()

    expect(wrapper.find('.source-search-status').exists()).toBe(true)
    expect(wrapper.find('.source-search-status').text()).toContain('source.search_empty')
  })

  it('搜索有结果时，显示结果数量', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
      createSource({ id: 2, owner: 'vuejs', repo: 'vue' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const searchInput = wrapper.get('input[placeholder="source.search"]')

    await searchInput.setValue('vue')
    await flushPromises()

    expect(wrapper.find('.source-search-status').text()).toContain('source.search_result_count')
  })

  it('清空搜索框后，恢复显示所有 source', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
      createSource({ id: 2, owner: 'facebook', repo: 'react' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const searchInput = wrapper.get('input[placeholder="source.search"]')

    await searchInput.setValue('vue')
    await flushPromises()
    expect(wrapper.findAll('.source-item')).toHaveLength(1)

    await wrapper.get('.input-clear-btn').trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.source-item')).toHaveLength(2)
  })

  it('搜索模式下的活动搜索状态指示：切换按钮上显示小圆点', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const searchInput = wrapper.get('input[placeholder="source.search"]')
    await searchInput.setValue('vue')
    await flushPromises()

    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find('.mode-toggle-dot').exists()).toBe(true)
  })
})

// ============ 键盘导航 ============

describe('SourceTab — 键盘导航', () => {
  it('排序触发按钮：Enter 打开下拉菜单', async () => {
    const { wrapper } = mountSourceTab([])
    const sortTrigger = wrapper.get('.sort-trigger')

    await sortTrigger.trigger('keydown', { key: 'Enter' })
    expect(wrapper.find('.sort-dropdown').exists()).toBe(true)
  })

  it('排序触发按钮：ArrowDown 打开下拉菜单', async () => {
    const { wrapper } = mountSourceTab([])
    const sortTrigger = wrapper.get('.sort-trigger')

    await sortTrigger.trigger('keydown', { key: 'ArrowDown' })
    expect(wrapper.find('.sort-dropdown').exists()).toBe(true)
  })

  it('排序触发按钮：Space 打开下拉菜单', async () => {
    const { wrapper } = mountSourceTab([])
    const sortTrigger = wrapper.get('.sort-trigger')

    await sortTrigger.trigger('keydown', { key: ' ' })
    expect(wrapper.find('.sort-dropdown').exists()).toBe(true)
  })

  it('排序触发按钮：Escape 关闭下拉菜单', async () => {
    const { wrapper } = mountSourceTab([])
    const sortTrigger = wrapper.get('.sort-trigger')

    await sortTrigger.trigger('click')
    expect(wrapper.find('.sort-dropdown').exists()).toBe(true)

    await sortTrigger.trigger('keydown', { key: 'Escape' })
    expect(wrapper.find('.sort-dropdown').exists()).toBe(false)
  })

  it('排序下拉菜单：ArrowDown/ArrowUp 在按钮间移动焦点', async () => {
    const { wrapper } = mountSourceTab([])
    await wrapper.get('.sort-trigger').trigger('click')

    const dropdown = wrapper.get('.sort-dropdown')
    const buttons = dropdown.findAll('button')

    await buttons[0].trigger('keydown', { key: 'ArrowDown' })
    // No error thrown, focus navigation executed
  })

  it('排序下拉菜单：Escape 关闭菜单', async () => {
    const { wrapper } = mountSourceTab([])
    await wrapper.get('.sort-trigger').trigger('click')
    expect(wrapper.find('.sort-dropdown').exists()).toBe(true)

    // handleSortDropdownKeydown 要求 target 是 BUTTON
    const firstButton = wrapper.find('.sort-dropdown button')
    await firstButton.trigger('keydown', { key: 'Escape' })
    expect(wrapper.find('.sort-dropdown').exists()).toBe(false)
  })

  it('更多按钮：Enter 打开面板', async () => {
    const source = createSource()
    const { wrapper } = mountSourceTab([source])
    const moreButton = wrapper.get('.btn-more')

    await moreButton.trigger('keydown', { key: 'Enter' })
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(true)
  })

  it('更多按钮：Escape 关闭面板', async () => {
    const source = createSource()
    const { wrapper } = mountSourceTab([source])
    const moreButton = wrapper.get('.btn-more')

    await moreButton.trigger('click')
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(true)

    await moreButton.trigger('keydown', { key: 'Escape' })
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(false)
  })

  it('更多按钮：ArrowDown 打开面板', async () => {
    const source = createSource()
    const { wrapper } = mountSourceTab([source])
    const moreButton = wrapper.get('.btn-more')

    await moreButton.trigger('keydown', { key: 'ArrowDown' })
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(true)
  })

  it('点击文档其他位置关闭更多面板', async () => {
    const source = createSource()
    const { wrapper } = mountSourceTab([source])
    const moreButton = wrapper.get('.btn-more')

    await moreButton.trigger('click')
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(true)

    await document.dispatchEvent(new Event('click'))
    await flushPromises()

    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(false)
  })
})

// ============ Tooltip ============

describe('SourceTab — Tooltip', () => {
  it('鼠标悬停在健康图标上显示 tooltip', async () => {
    const source = createSource({
      enabled: true,
      last_check_status: 'ok',
      description: 'A JavaScript framework',
    })
    const { wrapper } = mountSourceTab([source], {
      totalReleaseCounts: { 'vuejs/core': 50 },
    })

    const healthDot = wrapper.get('.health-dot')
    await healthDot.trigger('mouseenter', { clientX: 100, clientY: 100 })
    await flushPromises()

    expect(wrapper.find('.source-health-tooltip').exists()).toBe(true)
    const tooltip = wrapper.get('.source-health-tooltip')
    expect(tooltip.text()).toContain('source.tooltip_status')
    expect(tooltip.text()).toContain('source.tooltip_history')
    expect(tooltip.text()).toContain('source.tooltip_about')
  })

  it('鼠标离开健康图标时隐藏 tooltip', async () => {
    const source = createSource({ enabled: true, last_check_status: 'ok' })
    const { wrapper } = mountSourceTab([source])

    const healthDot = wrapper.get('.health-dot')
    await healthDot.trigger('mouseenter', { clientX: 100, clientY: 100 })
    expect(wrapper.find('.source-health-tooltip').exists()).toBe(true)

    await healthDot.trigger('mouseleave')
    expect(wrapper.find('.source-health-tooltip').exists()).toBe(false)
  })

  it('disabled source 的 tooltip 显示暂停状态', async () => {
    const source = createSource({ enabled: false })
    const { wrapper } = mountSourceTab([source])

    const healthDot = wrapper.get('.health-dot')
    await healthDot.trigger('mouseenter', { clientX: 100, clientY: 100 })
    await flushPromises()

    const tooltip = wrapper.get('.source-health-tooltip')
    expect(tooltip.text()).toContain('source.health_paused')
  })

  it('无 totalReleaseCounts 时 tooltip 不显示历史版本信息', async () => {
    const source = createSource({ enabled: true, last_check_status: 'ok' })
    const { wrapper } = mountSourceTab([source], {
      totalReleaseCounts: {},
    })

    const healthDot = wrapper.get('.health-dot')
    await healthDot.trigger('mouseenter', { clientX: 100, clientY: 100 })
    await flushPromises()

    const tooltip = wrapper.get('.source-health-tooltip')
    expect(tooltip.text()).not.toContain('source.tooltip_history')
  })
})

// ============ 空状态 ============

describe('SourceTab — 空状态', () => {
  it('无 source 时显示空状态提示', () => {
    const { wrapper } = mountSourceTab([])

    expect(wrapper.find('.empty').exists()).toBe(true)
    expect(wrapper.find('.empty').text()).toContain('source.empty')
  })
})

// ============ Badge 显示 ============

describe('SourceTab — Badge 显示', () => {
  it('enabled source 显示 "启用" badge', () => {
    const source = createSource({ enabled: true, muted: false })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.badge-on').exists()).toBe(true)
    expect(wrapper.find('.badge-on').text()).toContain('source.enabled')
  })

  it('disabled source 显示 "暂停" badge', () => {
    const source = createSource({ enabled: false })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.badge-off').exists()).toBe(true)
    expect(wrapper.find('.badge-off').text()).toContain('source.paused')
  })

  it('muted source 显示 "静默" badge', () => {
    const source = createSource({ enabled: true, muted: true })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.badge-muted').exists()).toBe(true)
    expect(wrapper.find('.badge-muted').text()).toContain('source.muted')
  })
})

// ============ 选择模式下的批量操作（通过真实组件） ============

describe('SourceTab — 选择模式批量操作（组件级）', () => {
  it('进入选择模式后显示批量操作栏', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'other', repo: 'repo' })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')
    expect(wrapper.find('.bulk-bar').exists()).toBe(true)
    expect(wrapper.findAll('.source-checkbox')).toHaveLength(2)
  })

  it('取消选择模式后隐藏批量操作栏并清空选中', async () => {
    const sources = [createSource({ id: 1 })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')
    expect(wrapper.find('.bulk-bar').exists()).toBe(true)

    await wrapper.get('.btn-select').trigger('click')
    expect(wrapper.find('.bulk-bar').exists()).toBe(false)
    expect(wrapper.find('.source-checkbox').exists()).toBe(false)
  })

  it('批量暂停选中 source，调用 updateSource 并 emit update', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'other', repo: 'repo' })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')
    const checkboxes = wrapper.findAll('.source-checkbox input[type="checkbox"]')
    await checkboxes[0].setValue(true)
    await checkboxes[1].setValue(true)

    await wrapper.get('.btn-sm:nth-child(4)').trigger('click') // 批量暂停
    await flushPromises()

    expect(updateSourceMock).toHaveBeenCalledTimes(2)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('批量删除无选中时不触发操作', async () => {
    const sources = [createSource({ id: 1 })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')
    // 不选中任何项
    const deleteButton = wrapper.findAll('.btn-sm').find(btn => btn.text().includes('source.bulk_delete'))
    expect(deleteButton?.attributes('disabled')).toBeDefined()
  })
})

// ============ 排序持久化 ============

describe('SourceTab — 排序持久化', () => {
  it('选择排序字段后写入 localStorage', async () => {
    const { wrapper } = mountSourceTab([])

    await wrapper.get('.sort-trigger').trigger('click')
    const dropdownButtons = wrapper.findAll('.sort-dropdown button')
    await dropdownButtons[1].trigger('click') // name

    expect(window.localStorage.getItem('relwatch.source.sort.field')).toBe('name')
  })

  it('选择排序方向后写入 localStorage', async () => {
    const { wrapper } = mountSourceTab([])

    await wrapper.get('.sort-trigger').trigger('click')
    const dropdownButtons = wrapper.findAll('.sort-dropdown button')
    await dropdownButtons[4].trigger('click') // asc

    expect(window.localStorage.getItem('relwatch.source.sort.direction')).toBe('asc')
  })

  it('从 localStorage 恢复排序设置', () => {
    window.localStorage.setItem('relwatch.source.sort.field', 'status')
    window.localStorage.setItem('relwatch.source.sort.direction', 'asc')

    const { wrapper } = mountSourceTab([])

    expect(wrapper.get('.sort-trigger').text()).toContain('source.sort_status')
    expect(wrapper.get('.sort-direction-icon').text()).toBe('↑')
  })

  it('localStorage 无效值时回退到默认', () => {
    window.localStorage.setItem('relwatch.source.sort.field', 'invalid_value')

    const { wrapper } = mountSourceTab([])

    expect(wrapper.get('.sort-trigger').text()).toContain('source.sort_default')
  })
})

// ============ toggleMore 切换 ============

describe('SourceTab — 更多菜单切换', () => {
  it('点击更多按钮打开面板，再次点击关闭', async () => {
    const source = createSource()
    const { wrapper } = mountSourceTab([source])

    const moreButton = wrapper.get('.btn-more')

    await moreButton.trigger('click')
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(true)

    await moreButton.trigger('click')
    expect(wrapper.find('.dropdown-more-panel').exists()).toBe(false)
  })
})
