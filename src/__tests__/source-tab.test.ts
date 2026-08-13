import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import SourceTab from '../components/SourceTab.vue'
import { ShowToastKey } from '../injection-keys'
import { t } from '../i18n'
import { createSource, type TestSource } from './helpers'
import { message, confirm } from '@tauri-apps/plugin-dialog'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ message: vi.fn(), confirm: vi.fn() }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))

// API 层：仅替换发起 IPC 的函数；parseSourceUrl / buildYoutubeConfig / 源类型注册表走真实实现
vi.mock('../api/sources', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/sources')>()
  return { ...actual, addSource: vi.fn(), removeSource: vi.fn(), updateSource: vi.fn() }
})
vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return { ...actual, checkSingleSource: vi.fn() }
})
vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, openReleaseUrl: vi.fn() }
})

import { addSource, removeSource, updateSource } from '../api/sources'
import { checkSingleSource } from '../api/releases'
import { openReleaseUrl } from '../api/client'

const addSourceMock = vi.mocked(addSource)
const removeSourceMock = vi.mocked(removeSource)
const updateSourceMock = vi.mocked(updateSource)
const checkSingleSourceMock = vi.mocked(checkSingleSource)
const openReleaseUrlMock = vi.mocked(openReleaseUrl)
const messageMock = vi.mocked(message)
const confirmMock = vi.mocked(confirm)

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
      showSourceTypeIcons: true,
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

// 添加模式输入框（真实 placeholder 文案）
function addInput(wrapper: ReturnType<typeof mountSourceTab>['wrapper']) {
  return wrapper.get(`input[placeholder="${t('source.placeholder')}"]`)
}
// 搜索模式输入框
function searchInput(wrapper: ReturnType<typeof mountSourceTab>['wrapper']) {
  return wrapper.get(`input[placeholder="${t('source.search')}"]`)
}

beforeEach(() => {
  vi.clearAllMocks()
  window.localStorage.clear()
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
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).toHaveBeenCalledWith('github', 'vuejs', 'core', undefined)
    expect((input.element as HTMLInputElement).value).toBe('')
  })

  it('输入 YouTube 频道时显示订阅内容复选框行，添加时携带 config', async () => {
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw')
    await flushPromises()

    // 复选框行应可见（视频/直播勾选、帖子禁用）
    const row = wrapper.get('.yt-subscribe-row')
    const boxes = row.findAll('input[type="checkbox"]')
    expect(boxes.length).toBe(3)
    expect((boxes[0].element as HTMLInputElement).checked).toBe(true)
    expect((boxes[1].element as HTMLInputElement).checked).toBe(true)
    expect((boxes[2].element as HTMLInputElement).disabled).toBe(true)

    // 取消勾选视频 → 添加时 config 只含直播
    await boxes[0].setValue(false)
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).toHaveBeenCalledWith(
      'youtube',
      'UCXuqSBlHAE6Xw-yeJA0Tunw',
      '',
      JSON.stringify({ videos: false, live: true, posts: false }),
    )
  })

  it('YouTube 源视频直播均未勾选时阻止添加并提示', async () => {
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('@handle')
    await flushPromises()
    const row = wrapper.get('.yt-subscribe-row')
    const boxes = row.findAll('input[type="checkbox"]')
    await boxes[0].setValue(false)
    await boxes[1].setValue(false)
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).not.toHaveBeenCalled()
    expect(messageMock).toHaveBeenCalledWith(t('source.require_subscribe'), expect.any(Object))
  })

  it('输入无效 URL 时，显示错误对话框而不调用 addSource', async () => {
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://example.com/foo')
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).not.toHaveBeenCalled()
    expect(messageMock).toHaveBeenCalledWith(t('source.invalid_url'), expect.any(Object))
  })

  it('输入已存在的 source 时，显示 toast 提示', async () => {
    const existingSource = createSource({ owner: 'vuejs', repo: 'core' })
    const { wrapper, showToast } = mountSourceTab([existingSource])
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).not.toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith(t('source.exists'))
  })

  it('addSource 返回 0 表示已存在，显示 toast', async () => {
    addSourceMock.mockResolvedValue(0)
    const { wrapper, showToast } = mountSourceTab([])
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(addSourceMock).toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith(t('source.exists'))
  })

  it('addSource 抛出错误时，显示错误对话框（含错误详情）', async () => {
    addSourceMock.mockRejectedValue(new Error('Network error'))
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)
    const addButton = wrapper.get('.btn-add-source')

    await input.setValue('https://github.com/vuejs/core')
    await addButton.trigger('click')
    await flushPromises()

    expect(messageMock).toHaveBeenCalledWith(
      expect.stringContaining('Network error'),
      expect.any(Object)
    )
  })

  it('添加成功后，emit update', async () => {
    addSourceMock.mockResolvedValue(99)
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)
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

    const input = addInput(wrapper)
    await input.setValue('https://github.com/vuejs/core')

    expect(addButton.attributes('disabled')).toBeUndefined()
  })

  it('Enter 键触发添加', async () => {
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)

    await input.setValue('https://github.com/vuejs/core')
    await input.trigger('keyup.enter')
    await flushPromises()

    expect(addSourceMock).toHaveBeenCalledWith('github', 'vuejs', 'core', undefined)
  })

  it('清空按钮清空输入框', async () => {
    const { wrapper } = mountSourceTab([])
    const input = addInput(wrapper)

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
      expect.stringContaining('Delete failed'),
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
      expect.stringContaining('Update failed'),
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

    expect(wrapper.find('.source-status').exists()).toBe(true)

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
          snooze_until: null, ai_summary: null, ai_importance: null, body_translated: null, extra_metadata: null, source_description: null },
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
        showSourceTypeIcons: true,
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
      expect.stringContaining('Check failed'),
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

  it('YouTube 源点击查看发布，emit 频道名而非 channel_id', async () => {
    const source = createSource({ source_type: 'youtube', owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw', repo: '', description: '时局眼' })
    const { wrapper } = mountSourceTab([source])

    const viewReleasesButton = wrapper.findAll('.btn-icon-link')[1]
    await viewReleasesButton.trigger('click')

    expect(wrapper.emitted('openReleases')?.[0]).toEqual(['时局眼'])
  })

  it('YouTube 源无频道名时，查看发布回退 channel_id', async () => {
    const source = createSource({ source_type: 'youtube', owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw', repo: '', description: null })
    const { wrapper } = mountSourceTab([source])

    const viewReleasesButton = wrapper.findAll('.btn-icon-link')[1]
    await viewReleasesButton.trigger('click')

    expect(wrapper.emitted('openReleases')?.[0]).toEqual(['UCXuqSBlHAE6Xw-yeJA0Tunw'])
  })

  it('有未读 release 时，显示待更新链接并点击 emit openUnreadReleases', async () => {
    const source = createSource({ id: 5, owner: 'vuejs', repo: 'core', enabled: true, last_check_status: 'ok' })
    const { wrapper } = mountSourceTab([source], {
      unreadReleaseCounts: { 'github|vuejs|core': 3 },
    })

    const pendingLink = wrapper.get('.source-pending-link')
    expect(pendingLink.text()).toContain(t('source.pending_updates', '3'))
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
    expect(wrapper.find('.source-health-label').text()).toContain(t('source.health_error'))
  })

  it('disabled 状态显示暂停图标', () => {
    const source = createSource({ enabled: false })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.health-paused').exists()).toBe(true)
    expect(wrapper.find('.source-health-label').text()).toContain(t('source.health_paused'))
  })

  it('last_check_status 为空时显示未知状态', () => {
    const source = createSource({ enabled: true, last_check_status: '' })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.health-unknown').exists()).toBe(true)
    expect(wrapper.find('.source-health-label').text()).toContain(t('source.health_unknown'))
  })

  it('显示最后检查时间文本', () => {
    const source = createSource({ last_checked_at: '2025-06-01T12:00:00Z' })
    const { wrapper } = mountSourceTab([source])

    const metaElements = wrapper.findAll('.source-health-meta')
    expect(metaElements.length).toBeGreaterThan(0)
  })

  it('从未检查时显示「从未检查」文本', () => {
    const source = createSource({ last_checked_at: null })
    const { wrapper } = mountSourceTab([source])

    const metaElements = wrapper.findAll('.source-health-meta')
    const lastCheckedText = metaElements.find(el => el.text().includes(t('source.never_checked')))
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
    const failureCount = metaElements.find(el => el.text().includes(t('source.failure_count', '5')))
    expect(failureCount).toBeTruthy()
  })

  it('error source 显示 last_check_message 经过 translateError（真实实现）', () => {
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
    const input = searchInput(wrapper)

    await input.setValue('vue')
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
    const input = searchInput(wrapper)

    await input.setValue('nonexistent')
    await flushPromises()

    expect(wrapper.find('.source-search-status').exists()).toBe(true)
    expect(wrapper.find('.source-search-status').text()).toContain(t('source.search_empty'))
  })

  it('搜索有结果时，显示结果数量', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
      createSource({ id: 2, owner: 'vuejs', repo: 'vue' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const input = searchInput(wrapper)

    await input.setValue('vue')
    await flushPromises()

    expect(wrapper.find('.source-search-status').text()).toContain(t('source.search_result_count', '2'))
  })

  it('搜索模式下显示计数徽标，数量随过滤变化', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
      createSource({ id: 2, owner: 'vuejs', repo: 'vue' }),
      createSource({ id: 3, owner: 'facebook', repo: 'react' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    // 默认添加模式：不显示徽标
    expect(wrapper.find('.source-count').exists()).toBe(false)

    // 切到搜索模式：显示总数，徽标贴右缘（无 has-clear）
    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find('.source-count').exists()).toBe(true)
    expect(wrapper.find('.source-count').text()).toBe('(3)')
    expect(wrapper.find('.source-count').attributes('title')).toBe(t('source.search_result_count', '3'))
    expect(wrapper.find('.source-count').classes()).not.toContain('has-clear')

    // 输入关键词过滤：数量更新，徽标让位给清空按钮
    const input = searchInput(wrapper)
    await input.setValue('vue')
    await flushPromises()
    expect(wrapper.find('.source-count').text()).toBe('(2)')
    expect(wrapper.find('.source-count').classes()).toContain('has-clear')

    // 切回添加模式：徽标隐藏
    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find('.source-count').exists()).toBe(false)
  })

  it('清空搜索框后，恢复显示所有 source', async () => {
    const sources = [
      createSource({ id: 1, owner: 'vuejs', repo: 'core' }),
      createSource({ id: 2, owner: 'facebook', repo: 'react' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-mode-toggle').trigger('click')
    const input = searchInput(wrapper)

    await input.setValue('vue')
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
    const input = searchInput(wrapper)
    await input.setValue('vue')
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
      totalReleaseCounts: { 'github|vuejs|core': 50 },
    })

    const healthDot = wrapper.get('.health-dot')
    await healthDot.trigger('mouseenter', { clientX: 100, clientY: 100 })
    await flushPromises()

    expect(wrapper.find('.source-health-tooltip').exists()).toBe(true)
    const tooltip = wrapper.get('.source-health-tooltip')
    expect(tooltip.text()).toContain(t('source.tooltip_status'))
    expect(tooltip.text()).toContain(t('source.tooltip_history'))
    expect(tooltip.text()).toContain(t('source.tooltip_about'))
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
    expect(tooltip.text()).toContain(t('source.health_paused'))
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
    expect(tooltip.text()).not.toContain(t('source.tooltip_history'))
  })
})

// ============ 空状态 ============

describe('SourceTab — 空状态', () => {
  it('无 source 时显示空状态提示', () => {
    const { wrapper } = mountSourceTab([])

    expect(wrapper.find('.empty').exists()).toBe(true)
    expect(wrapper.find('.empty').text()).toContain(t('source.empty'))
  })
})

// ============ Badge 显示 ============

describe('SourceTab — Badge 显示', () => {
  it('enabled source 显示「启用」badge', () => {
    const source = createSource({ enabled: true, muted: false })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.source-status-on').exists()).toBe(true)
    expect(wrapper.find('.source-status-on').text()).toContain(t('source.enabled'))
  })

  it('disabled source 显示「暂停」badge', () => {
    const source = createSource({ enabled: false })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.source-status-off').exists()).toBe(true)
    expect(wrapper.find('.source-status-off').text()).toContain(t('source.paused'))
  })

  it('muted source 显示「静默」badge', () => {
    const source = createSource({ enabled: true, muted: true })
    const { wrapper } = mountSourceTab([source])

    expect(wrapper.find('.source-status').exists()).toBe(true)
    expect(wrapper.find('.source-status').text()).toContain(t('source.muted'))
  })
})

// ============ 选择模式下的批量操作 ============

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

    const pauseBtn = wrapper.findAll('.bulk-bar .btn-sm').find(b => b.text() === t('source.bulk_pause'))!
    await pauseBtn.trigger('click')
    await flushPromises()

    expect(updateSourceMock).toHaveBeenCalledTimes(2)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('批量静默/取消静默选中 source（按钮入口）', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'other', repo: 'repo' })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')
    const checkboxes = wrapper.findAll('.source-checkbox input[type="checkbox"]')
    await checkboxes[0].setValue(true)
    await checkboxes[1].setValue(true)

    const muteBtn = wrapper.findAll('.bulk-bar .btn-sm').find(b => b.text() === t('source.bulk_mute'))!
    await muteBtn.trigger('click')
    await flushPromises()
    expect(updateSourceMock).toHaveBeenCalledWith(1, true, 60, true)
    expect(updateSourceMock).toHaveBeenCalledWith(2, true, 60, true)

    const unmuteBtn = wrapper.findAll('.bulk-bar .btn-sm').find(b => b.text() === t('source.bulk_unmute'))!
    await unmuteBtn.trigger('click')
    await flushPromises()
    expect(updateSourceMock).toHaveBeenCalledWith(1, true, 60, false)
    expect(updateSourceMock).toHaveBeenCalledWith(2, true, 60, false)
  })

  it('批量操作无选中时不触发（按钮禁用）', async () => {
    const sources = [createSource({ id: 1 })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')

    const deleteButton = wrapper.findAll('.bulk-bar .btn-sm').find(b => b.text() === t('source.bulk_delete'))!
    expect(deleteButton.attributes('disabled')).toBeDefined()
  })

  it('批量删除经确认后调用 removeSource', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'other', repo: 'repo' })]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.btn-select').trigger('click')
    const checkboxes = wrapper.findAll('.source-checkbox input[type="checkbox"]')
    await checkboxes[0].setValue(true)
    await checkboxes[1].setValue(true)

    const deleteButton = wrapper.findAll('.bulk-bar .btn-sm').find(b => b.text() === t('source.bulk_delete'))!
    await deleteButton.trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalled()
    expect(removeSourceMock).toHaveBeenCalledTimes(2)
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
    await dropdownButtons[5].trigger('click') // asc（前 5 个是排序字段，后 2 个是方向）

    expect(window.localStorage.getItem('relwatch.source.sort.direction')).toBe('asc')
  })

  it('从 localStorage 恢复排序设置', () => {
    window.localStorage.setItem('relwatch.source.sort.field', 'status')
    window.localStorage.setItem('relwatch.source.sort.direction', 'asc')

    const { wrapper } = mountSourceTab([])

    expect(wrapper.get('.sort-trigger').text()).toContain(t('source.sort_status'))
    expect(wrapper.get('.sort-direction-icon').text()).toBe('↑')
  })

  it('localStorage 无效值时回退到默认', () => {
    window.localStorage.setItem('relwatch.source.sort.field', 'invalid_value')

    const { wrapper } = mountSourceTab([])

    expect(wrapper.get('.sort-trigger').text()).toContain(t('source.sort_default'))
  })
})

// ============ 按来源排序 ============

describe('SourceTab — 按来源排序', () => {
  it('选择「来源」排序后按注册表顺序排列，同类型内按名称', async () => {
    // 显式指定升序，避免默认（desc）干扰断言
    window.localStorage.setItem('relwatch.source.sort.direction', 'asc')
    const sources = [
      createSource({ id: 1, source_type: 'bilibili', owner: '12345', repo: '', description: '某UP主' }),
      createSource({ id: 2, source_type: 'youtube', owner: 'UCzzz', repo: '', description: '某频道' }),
      createSource({ id: 3, source_type: 'github', owner: 'vuejs', repo: 'core' }),
      createSource({ id: 4, source_type: 'huggingface', owner: 'moonshotai', repo: '' }),
      createSource({ id: 5, source_type: 'github', owner: 'tauri-apps', repo: 'tauri' }),
    ]
    const { wrapper } = mountSourceTab(sources)

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[3].trigger('click') // type

    const badges = wrapper.findAll('.source-type-badge')
    // 注册表顺序：github → huggingface → youtube → bilibili；github 内部按名称 tauri-apps → vuejs（asc）
    expect(badges.map(b => b.classes().find(c => ['github', 'huggingface', 'youtube', 'bilibili'].includes(c)))).toEqual([
      'github', 'github', 'huggingface', 'youtube', 'bilibili',
    ])
    const items = wrapper.findAll('.source-item')
    expect(items[0].text()).toContain('tauri-apps')
    expect(items[1].text()).toContain('vuejs')
    expect(items[2].text()).toContain('moonshotai')
    expect(items[3].text()).toContain('某频道')
    expect(items[4].text()).toContain('某UP主')
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

// ============ YouTube 源显示名 ============

describe('SourceTab — YouTube 源显示名', () => {
  it('youtube 源显示频道名（description）而非 channel_id', () => {
    const { wrapper } = mountSourceTab([createSource({ source_type: 'youtube', owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw', repo: '', description: 'Videos' })])
    expect(wrapper.get('.source-name').text()).toBe('Videos')
  })

  it('youtube 源兼容旧版 "YouTube channel: " 前缀描述', () => {
    const { wrapper } = mountSourceTab([createSource({ source_type: 'youtube', owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw', repo: '', description: 'YouTube channel: Videos' })])
    expect(wrapper.get('.source-name').text()).toBe('Videos')
  })

  it('youtube 源无描述时回退 owner', () => {
    const { wrapper } = mountSourceTab([createSource({ source_type: 'youtube', owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw', repo: '', description: null })])
    expect(wrapper.get('.source-name').text()).toBe('UCXuqSBlHAE6Xw-yeJA0Tunw')
  })

  it('github 源仍显示 owner/repo', () => {
    const { wrapper } = mountSourceTab([createSource({ source_type: 'github', owner: 'vuejs', repo: 'core' })])
    expect(wrapper.get('.source-name').text()).toBe('vuejs/core')
  })
})
