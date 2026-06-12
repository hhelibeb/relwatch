import { describe, it, expect, vi, beforeEach } from 'vitest'
import { shallowMount } from '@vue/test-utils'
import ReleaseItem from '../components/ReleaseItem.vue'
import { ShowToastKey } from '../injection-keys'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../api/releases', () => ({
  setNotificationState: vi.fn(),
  deleteRelease: vi.fn(),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
}))

vi.mock('../utils', () => ({
  formatDate: vi.fn(() => '2024-06-15'),
  isReadStatus: vi.fn((status: string) => status === 'clicked' || status === 'ignored'),
  isUnreadStatus: vi.fn((status: string, snoozeUntil?: string | null) => {
    if (status === 'snoozed' && snoozeUntil) {
      const until = new Date(snoozeUntil)
      return isNaN(until.getTime()) || until.getTime() <= Date.now()
    }
    return status === 'pending' || status === 'snoozed'
  }),
  statusClass: vi.fn((status: string) => {
    if (status === 'pending' || status === 'snoozed') return 'status-unread'
    return 'status-read'
  }),
  statusLabel: vi.fn((status: string) => status),
}))

vi.mock('../composables/contextMenuBus', () => ({
  registerCloser: vi.fn(),
  unregisterCloser: vi.fn(),
  closeAllContextMenus: vi.fn(),
}))

import { setNotificationState, deleteRelease } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { registerCloser, unregisterCloser } from '../composables/contextMenuBus'

function createRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'tauri-apps',
    repo: 'tauri',
    tag_name: 'v2.0.0',
    release_name: 'Tauri 2.0 Stable',
    html_url: 'https://github.com/tauri-apps/tauri/releases/tag/v2.0.0',
    published_at: '2025-06-01T00:00:00Z',
    prerelease: false,
    body: null,
    detected_at: '2025-06-01T00:00:00Z',
    notification_status: 'pending',
    snooze_until: null,
    ai_summary: null,
    ai_importance: null,
    ...overrides,
  }
}

function mountRelease(release: ReleaseInfo) {
  return shallowMount(ReleaseItem, {
    props: { release },
    global: {
      provide: {
        [ShowToastKey as symbol]: vi.fn(),
      },
    },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
})

/**
 * ReleaseItem.vue 真实运行场景测试
 *
 * 版本列表中的单个版本卡片，提供：
 * - 版本信息展示（仓库/标签/标题/日期/状态/预发布标记）
 * - AI 摘要与截断悬浮提示
 * - 右键菜单（打开链接/复制链接/删除）和摘要右键菜单（复制摘要）
 * - 通知状态变更（点击链接自动标记、稍后提醒、忽略）
 */
describe('ReleaseItem.vue — 渲染', () => {
  it('显示仓库名和标签名', () => {
    const wrapper = mountRelease(createRelease())

    expect(wrapper.text()).toContain('tauri-apps/tauri')
    expect(wrapper.text()).toContain('v2.0.0')
  })

  it('release_name 与 tag_name 不同时显示标题', () => {
    const wrapper = mountRelease(createRelease({ release_name: 'Tauri 2.0 Stable' }))

    expect(wrapper.text()).toContain('Tauri 2.0 Stable')
  })

  it('release_name 等于 tag_name 时不显示标题', () => {
    const wrapper = mountRelease(createRelease({ release_name: 'v2.0.0' }))

    expect(wrapper.find('.release-title').exists()).toBe(false)
  })

  it('预发布版显示 prerelease badge', () => {
    const wrapper = mountRelease(createRelease({ prerelease: true }))

    expect(wrapper.text()).toContain('release.prerelease')
  })

  it('正式版不显示 prerelease badge', () => {
    const wrapper = mountRelease(createRelease({ prerelease: false }))

    expect(wrapper.text()).not.toContain('release.prerelease')
  })

  it('snoozed 且 snooze_until 有值时显示提醒时间', () => {
    const wrapper = mountRelease(createRelease({
      notification_status: 'snoozed',
      snooze_until: '2025-06-10T00:00:00Z',
    }))

    expect(wrapper.text()).toContain('release.snooze_until')
  })

  it('AI 摘要存在时渲染摘要行', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '修复了关键错误' }))

    expect(wrapper.find('.release-summary-text').exists()).toBe(true)
    expect(wrapper.text()).toContain('修复了关键错误')
  })

  it('无 AI 摘要时不显示摘要行', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: null }))

    expect(wrapper.find('.release-summary-line').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 通知状态操作', () => {
  it('未读版本（pending）上显示 Ignore 按钮', () => {
    const wrapper = mountRelease(createRelease({ notification_status: 'pending' }))

    expect(wrapper.text()).toContain('release.ignore')
  })

  it('已读版本（clicked）上显示 Snooze 按钮，无 Ignore', () => {
    const wrapper = mountRelease(createRelease({ notification_status: 'clicked' }))

    expect(wrapper.text()).toContain('release.snooze')
    expect(wrapper.text()).not.toContain('release.ignore')
  })

  it('已读版本（ignored）上显示 Snooze 按钮', () => {
    const wrapper = mountRelease(createRelease({ notification_status: 'ignored' }))

    expect(wrapper.text()).toContain('release.snooze')
  })

  it('点击 Ignore 调用 setNotificationState("ignored")', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 10, notification_status: 'pending' }))

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setNotificationState).toHaveBeenCalledWith(10, 'ignored', undefined)
  })

  it('点击 Ignore 后 emit update', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 10, notification_status: 'pending' }))

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('状态更新失败时调用 Toast 报错', async () => {
    const toast = vi.fn()
    vi.mocked(setNotificationState).mockRejectedValue(new Error('err.network'))

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 10, notification_status: 'pending' }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(expect.stringContaining('release.status_failed'))
  })

  it('未读版本点击链接 → 打开 URL + 设置 clicked + emit update', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    vi.mocked(openReleaseUrl).mockResolvedValue(undefined)

    const wrapper = mountRelease(createRelease({
      id: 5,
      html_url: 'https://github.com/test/repo/releases/tag/v1.0.0',
      notification_status: 'pending',
    }))

    await wrapper.find('.release-link-action').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(openReleaseUrl).toHaveBeenCalledWith('https://github.com/test/repo/releases/tag/v1.0.0')
    expect(setNotificationState).toHaveBeenCalledWith(5, 'clicked')
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('已读版本点击链接 → 只打开 URL，不设置状态', async () => {
    vi.mocked(openReleaseUrl).mockResolvedValue(undefined)

    const wrapper = mountRelease(createRelease({
      id: 6,
      html_url: 'https://github.com/test/repo/releases/tag/v1.0.0',
      notification_status: 'clicked',
    }))

    await wrapper.find('.release-link-action').trigger('click')

    expect(openReleaseUrl).toHaveBeenCalledWith('https://github.com/test/repo/releases/tag/v1.0.0')
    expect(setNotificationState).not.toHaveBeenCalled()
  })

  it('Snooze 按钮调用 setNotificationState("snoozed", 1440)', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 7, notification_status: 'clicked' }))

    const snoozeBtn = wrapper.findAll('button').find(b => b.text().includes('release.snooze'))
    expect(snoozeBtn).toBeTruthy()
    await snoozeBtn!.trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setNotificationState).toHaveBeenCalledWith(7, 'snoozed', 1440)
  })
})

describe('ReleaseItem.vue — 生命周期', () => {
  it('挂载时注册 contextMenuBus', () => {
    mountRelease(createRelease())

    expect(registerCloser).toHaveBeenCalled()
  })

  it('卸载时注销 contextMenuBus', () => {
    const wrapper = mountRelease(createRelease())
    wrapper.unmount()

    expect(unregisterCloser).toHaveBeenCalled()
  })
})

describe('ReleaseItem.vue — 重要性样式', () => {
  it('ai_importance="大" → release-importance-high', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: '大' }))

    expect(wrapper.find('.release-item').classes()).toContain('release-importance-high')
  })

  it('ai_importance="中" → release-importance-medium', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: '中' }))

    expect(wrapper.find('.release-item').classes()).toContain('release-importance-medium')
  })

  it('ai_importance="小" → release-importance-low', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: '小' }))

    expect(wrapper.find('.release-item').classes()).toContain('release-importance-low')
  })

  it('无重要性时不加重要性 class', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: null }))

    const classes = wrapper.find('.release-item').classes()
    expect(classes).not.toContain('release-importance-high')
    expect(classes).not.toContain('release-importance-medium')
    expect(classes).not.toContain('release-importance-low')
  })
})
