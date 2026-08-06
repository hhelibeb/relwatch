import { describe, it, expect, vi, beforeEach } from 'vitest'
import { shallowMount } from '@vue/test-utils'
import ReleaseItem from '../components/ReleaseItem.vue'
import { ShowToastKey } from '../injection-keys'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../api/releases', () => ({
  setNotificationState: vi.fn(),
  deleteRelease: vi.fn(),
  translateRelease: vi.fn(),
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
import { closeAllContextMenus } from '../composables/contextMenuBus'

// Mock clipboard
const mockClipboard = { writeText: vi.fn().mockResolvedValue(undefined) }
Object.assign(navigator, { clipboard: mockClipboard })

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
    body_translated: null,
    extra_metadata: null,
    source_description: null,
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
 * ReleaseItem.vue 补充测试 — 覆盖真实使用场景:
 * - 右键菜单交互(复制链接/打开链接/删除版本)
 * - 摘要悬浮提示(mouseenter/focus/move)
 * - 摘要右键菜单(复制摘要)
 * - 状态操作成功消息(snooze/ignore)
 * - 删除失败处理
 * - 组件卸载清理(document 事件)
 */

describe('ReleaseItem.vue — 右键菜单: 版本链接', () => {
  it('右键点击版本链接打开上下文菜单', async () => {
    const wrapper = mountRelease(createRelease({ id: 42, html_url: 'https://example.com/release' }))

    const linkBtn = wrapper.find('.release-link-action')
    await linkBtn.trigger('contextmenu', { clientX: 100, clientY: 200 })

    expect(closeAllContextMenus).toHaveBeenCalled()
    // 应该渲染 ContextMenu 组件
    expect(wrapper.findComponent({ name: 'ContextMenu' }).exists()).toBe(true)
  })

  it('右键菜单选择"打开链接" → 调用 openReleaseUrl', async () => {
    const wrapper = mountRelease(createRelease({ html_url: 'https://example.com/release' }))

    // 打开右键菜单
    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    // 找到 ContextMenu 并触发 action
    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'openLink')

    expect(openReleaseUrl).toHaveBeenCalledWith('https://example.com/release')
  })

  it('右键菜单选择"复制链接" → 写入剪贴板', async () => {
    const wrapper = mountRelease(createRelease({ html_url: 'https://example.com/release' }))

    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'copyLink')

    expect(mockClipboard.writeText).toHaveBeenCalledWith('https://example.com/release')
  })

  it('右键菜单选择"删除版本" → 调用 deleteRelease 并 emit update', async () => {
    vi.mocked(deleteRelease).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 99 }))

    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'deleteRelease')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(deleteRelease).toHaveBeenCalledWith(99)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('删除版本失败时显示错误 Toast', async () => {
    const toast = vi.fn()
    vi.mocked(deleteRelease).mockRejectedValue(new Error('permission denied'))

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 99 }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'deleteRelease')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(expect.stringContaining('release.delete_failed'))
    expect(toast).toHaveBeenCalledWith(expect.stringContaining('permission denied'))
  })

  it('右键菜单关闭时调用 closeMenus', async () => {
    const wrapper = mountRelease(createRelease())

    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('close')

    // 菜单应该关闭(组件不再渲染)
    expect(wrapper.findComponent({ name: 'ContextMenu' }).exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 右键菜单: 摘要复制', () => {
  it('右键点击摘要打开摘要上下文菜单', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '这是一个修复摘要' }))

    const summaryEl = wrapper.find('.release-summary-text')
    await summaryEl.trigger('contextmenu', { clientX: 150, clientY: 250 })

    expect(closeAllContextMenus).toHaveBeenCalled()
    // 应该有 ContextMenu 组件
    expect(wrapper.findComponent({ name: 'ContextMenu' }).exists()).toBe(true)
  })

  it('摘要右键菜单选择"复制内容" → 写入剪贴板', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '这是一个修复摘要' }))

    await wrapper.find('.release-summary-text').trigger('contextmenu', { clientX: 150, clientY: 250 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'copyContent')

    expect(mockClipboard.writeText).toHaveBeenCalledWith('这是一个修复摘要')
  })

  it('无摘要时右键不触发菜单', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: null }))

    // 没有摘要行
    expect(wrapper.find('.release-summary-text').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 摘要悬浮提示', () => {
  it('mouseenter 摘要时如果文本被截断则显示提示', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '很长的摘要内容'.repeat(50) }))

    const summaryEl = wrapper.find('.release-summary-text')

    // 模拟 scrollHeight > clientHeight (文本被截断)
    Object.defineProperty(summaryEl.element, 'scrollHeight', { value: 100, configurable: true })
    Object.defineProperty(summaryEl.element, 'clientHeight', { value: 40, configurable: true })

    await summaryEl.trigger('mouseenter', { clientX: 200, clientY: 300 })

    // 应该显示 tooltip
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(true)
    expect(wrapper.find('.release-summary-tooltip').text()).toContain('很长的摘要内容')
  })

  it('mouseenter 摘要时如果文本未截断则不显示提示', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '短摘要' }))

    const summaryEl = wrapper.find('.release-summary-text')

    // 模拟 scrollHeight <= clientHeight (文本未截断)
    Object.defineProperty(summaryEl.element, 'scrollHeight', { value: 20, configurable: true })
    Object.defineProperty(summaryEl.element, 'clientHeight', { value: 40, configurable: true })

    await summaryEl.trigger('mouseenter', { clientX: 200, clientY: 300 })

    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(false)
  })

  it('mouseleave 摘要时隐藏提示', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '很长的摘要内容'.repeat(50) }))

    const summaryEl = wrapper.find('.release-summary-text')
    Object.defineProperty(summaryEl.element, 'scrollHeight', { value: 100, configurable: true })
    Object.defineProperty(summaryEl.element, 'clientHeight', { value: 40, configurable: true })

    await summaryEl.trigger('mouseenter', { clientX: 200, clientY: 300 })
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(true)

    await summaryEl.trigger('mouseleave')
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(false)
  })

  it('mousemove 时更新提示位置', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '很长的摘要内容'.repeat(50) }))

    const summaryEl = wrapper.find('.release-summary-text')
    Object.defineProperty(summaryEl.element, 'scrollHeight', { value: 100, configurable: true })
    Object.defineProperty(summaryEl.element, 'clientHeight', { value: 40, configurable: true })

    await summaryEl.trigger('mouseenter', { clientX: 200, clientY: 300 })
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(true)

    // 移动鼠标
    await summaryEl.trigger('mousemove', { clientX: 250, clientY: 350 })

    const tooltip = wrapper.find('.release-summary-tooltip')
    expect(tooltip.exists()).toBe(true)
  })

  it('focus 摘要时如果文本被截断则显示提示', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '很长的摘要内容'.repeat(50) }))

    const summaryEl = wrapper.find('.release-summary-text')
    Object.defineProperty(summaryEl.element, 'scrollHeight', { value: 100, configurable: true })
    Object.defineProperty(summaryEl.element, 'clientHeight', { value: 40, configurable: true })
    // Mock getBoundingClientRect for focus
    summaryEl.element.getBoundingClientRect = vi.fn().mockReturnValue({
      left: 100,
      bottom: 140,
    })

    await summaryEl.trigger('focus')

    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(true)
  })

  it('blur 摘要时隐藏提示', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '很长的摘要内容'.repeat(50) }))

    const summaryEl = wrapper.find('.release-summary-text')
    Object.defineProperty(summaryEl.element, 'scrollHeight', { value: 100, configurable: true })
    Object.defineProperty(summaryEl.element, 'clientHeight', { value: 40, configurable: true })

    await summaryEl.trigger('mouseenter', { clientX: 200, clientY: 300 })
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(true)

    await summaryEl.trigger('blur')
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 状态操作成功消息', () => {
  it('点击 Snooze 成功后显示"snooze_scheduled" Toast', async () => {
    const toast = vi.fn()
    vi.mocked(setNotificationState).mockResolvedValue(undefined)

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 10, notification_status: 'clicked' }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    const snoozeBtn = wrapper.findAll('button').find(b => b.text().includes('release.snooze'))
    await snoozeBtn!.trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith('release.snooze_scheduled')
  })

  it('点击 Ignore 成功后显示"notification_cancelled" Toast', async () => {
    const toast = vi.fn()
    vi.mocked(setNotificationState).mockResolvedValue(undefined)

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 10, notification_status: 'pending' }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith('release.notification_cancelled')
  })
})

describe('ReleaseItem.vue — 显示辅助函数', () => {
  it('release_name 为空字符串时不显示标题', () => {
    const wrapper = mountRelease(createRelease({ release_name: '' }))

    expect(wrapper.find('.release-title').exists()).toBe(false)
  })

  it('release_name 只有空白字符时不显示标题', () => {
    const wrapper = mountRelease(createRelease({ release_name: '   ' }))

    expect(wrapper.find('.release-title').exists()).toBe(false)
  })

  it('有 ai_importance 时显示重要性标签', () => {
    const wrapper = mountRelease(createRelease({
      ai_summary: '修复bug',
      ai_importance: '大',
    }))

    expect(wrapper.find('.release-importance-chip').exists()).toBe(true)
    // 组件将中文枚举映射为 i18n key（mock 的 t 原样返回 key）
    expect(wrapper.find('.release-importance-chip').text()).toBe('release.importance_high')
  })

  it('无 ai_importance 时不显示重要性标签', () => {
    const wrapper = mountRelease(createRelease({
      ai_summary: '修复bug',
      ai_importance: null,
    body_translated: null,
    extra_metadata: null,
    source_description: null,
    }))

    expect(wrapper.find('.release-importance-chip').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 操作按钮状态', () => {
  it('操作中(isUpdating)时所有按钮 disabled', async () => {
    vi.mocked(setNotificationState).mockReturnValue(new Promise(() => {})) // 永不 resolve

    const wrapper = mountRelease(createRelease({ notification_status: 'pending' }))

    // 点击 ignore 触发 isUpdating
    wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    // 所有按钮应该被禁用
    const buttons = wrapper.findAll('button')
    for (const btn of buttons) {
      expect((btn.element as HTMLButtonElement).disabled).toBe(true)
    }
  })

  it('snoozed 状态且 snooze_until 过期时显示 Ignore 按钮', () => {
    // snooze_until 已过期
    const wrapper = mountRelease(createRelease({
      notification_status: 'snoozed',
      snooze_until: '2020-01-01T00:00:00Z',
    }))

    expect(wrapper.text()).toContain('release.ignore')
  })

  it('snoozed 状态且 snooze_until 未到期时仍显示 Ignore 按钮（闭环不中断）', () => {
    // snooze_until 未来，按钮判断不传 snooze_until 以保证可取消提醒
    const wrapper = mountRelease(createRelease({
      notification_status: 'snoozed',
      snooze_until: '2099-01-01T00:00:00Z',
    }))

    expect(wrapper.text()).toContain('release.ignore')
  })
})

describe('ReleaseItem.vue — 组件卸载清理', () => {
  it('卸载时移除 document click 事件监听器', () => {
    const removeSpy = vi.spyOn(document, 'removeEventListener')

    const wrapper = mountRelease(createRelease())
    wrapper.unmount()

    expect(removeSpy).toHaveBeenCalledWith('click', expect.any(Function))
    removeSpy.mockRestore()
  })
})
