import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { shallowMount } from '@vue/test-utils'
import { ref } from 'vue'
import { t } from '../i18n'
import ReleaseItem from '../components/ReleaseItem.vue'
import { ShowToastKey, ShowImportanceKey } from '../injection-keys'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../api/releases', () => ({
  setNotificationState: vi.fn(),
  deleteRelease: vi.fn(),
  translateRelease: vi.fn(),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  copyTextToClipboard: vi.fn(),
}))

// 仅替换 formatDate（jsdom 无时区/本地化渲染）；isUnreadStatus/statusClass/statusLabel
// 必须引用 utils 真实实现——在本文件复制实现会让测试按旧语义放行
vi.mock('../utils', async importOriginal => {
  const actual = await importOriginal<typeof import('../utils')>()
  return {
    ...actual,
    formatDate: vi.fn(() => '2024-06-15'),
  }
})

vi.mock('../composables/contextMenuBus', () => ({
  registerCloser: vi.fn(),
  unregisterCloser: vi.fn(),
  closeAllContextMenus: vi.fn(),
  // 失效期开关：用例里按需 mockReturnValue(true) 模拟「窗口刚隐藏/失焦回来」
  isTransientUiInvalidated: vi.fn(() => false),
}))

import { setNotificationState, deleteRelease } from '../api/releases'
import { openReleaseUrl, copyTextToClipboard } from '../api/client'
import { closeAllContextMenus, isTransientUiInvalidated } from '../composables/contextMenuBus'

const isTransientUiInvalidatedMock = vi.mocked(isTransientUiInvalidated)

// 剪贴板写入统一走 Rust 路径（src/api/client.ts 的 copyTextToClipboard）
const copyTextToClipboardMock = vi.mocked(copyTextToClipboard)

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
    flag: 0,
    version_bump: null,
    ...overrides,
  }
}

function mountRelease(release: ReleaseInfo, provideOverrides: Record<symbol, unknown> = {}) {
  return shallowMount(ReleaseItem, {
    props: { release },
    global: {
      provide: {
        [ShowToastKey as symbol]: vi.fn(),
        ...provideOverrides,
      },
    },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
})

/**
 * ReleaseItem.vue 补充测试：右键菜单、摘要悬浮提示、状态操作 Toast、卸载清理等真实使用场景。
 */

describe('ReleaseItem.vue — 右键菜单: 版本链接', () => {
  it('右键点击版本链接打开上下文菜单', async () => {
    const wrapper = mountRelease(createRelease({ id: 42, html_url: 'https://example.com/release' }))

    const linkBtn = wrapper.find('.release-link-action')
    await linkBtn.trigger('contextmenu', { clientX: 100, clientY: 200 })

    expect(closeAllContextMenus).toHaveBeenCalled()
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

    expect(copyTextToClipboardMock).toHaveBeenCalledWith('https://example.com/release')
  })

  it('右键菜单选择"复制链接"失败时显示错误 Toast（不再静默失败）', async () => {
    const toast = vi.fn()
    copyTextToClipboardMock.mockRejectedValueOnce(new Error('clipboard busy'))
    const wrapper = mountRelease(createRelease({ html_url: 'https://example.com/release' }), {
      [ShowToastKey as symbol]: toast,
    })

    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'copyLink')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(expect.stringContaining(t('release.copy_failed')))
    expect(toast).toHaveBeenCalledWith(expect.stringContaining('clipboard busy'))
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

    expect(toast).toHaveBeenCalledWith(expect.stringContaining(t('release.delete_failed')))
    expect(toast).toHaveBeenCalledWith(expect.stringContaining('permission denied'))
  })

  it('右键菜单关闭时调用 closeMenus', async () => {
    const wrapper = mountRelease(createRelease())

    await wrapper.find('.release-link-action').trigger('contextmenu', { clientX: 100, clientY: 200 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('close')

    expect(wrapper.findComponent({ name: 'ContextMenu' }).exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 右键菜单: 摘要复制', () => {
  it('右键点击摘要打开摘要上下文菜单', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '这是一个修复摘要' }))

    const summaryEl = wrapper.find('.release-summary-text')
    await summaryEl.trigger('contextmenu', { clientX: 150, clientY: 250 })

    expect(closeAllContextMenus).toHaveBeenCalled()
    expect(wrapper.findComponent({ name: 'ContextMenu' }).exists()).toBe(true)
  })

  it('摘要右键菜单选择"复制内容" → 写入剪贴板', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '这是一个修复摘要' }))

    await wrapper.find('.release-summary-text').trigger('contextmenu', { clientX: 150, clientY: 250 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'copyContent')

    expect(copyTextToClipboardMock).toHaveBeenCalledWith('这是一个修复摘要')
  })

  it('摘要复制失败时显示错误 Toast（不再静默失败）', async () => {
    const toast = vi.fn()
    copyTextToClipboardMock.mockRejectedValueOnce(new Error('clipboard busy'))
    const wrapper = mountRelease(createRelease({ ai_summary: '这是一个修复摘要' }), {
      [ShowToastKey as symbol]: toast,
    })

    await wrapper.find('.release-summary-text').trigger('contextmenu', { clientX: 150, clientY: 250 })

    const ctxMenu = wrapper.findComponent({ name: 'ContextMenu' })
    await ctxMenu.vm.$emit('action', 'copyContent')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(expect.stringContaining(t('release.copy_failed')))
    expect(toast).toHaveBeenCalledWith(expect.stringContaining('clipboard busy'))
  })

  it('无摘要时右键不触发菜单', async () => {
    const wrapper = mountRelease(createRelease({ ai_summary: null }))

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

/**
 * 窗口隐藏到托盘（window.hide() = Win32 SW_HIDE）后，Chromium 会在窗口重新显示时
 * 把「隐藏前获得焦点的元素」重放一次 focus（实测 SW_HIDE → SW_SHOW 收到
 * WINDOW focus + target focus，不伴随任何鼠标事件）。
 * 失效期（useTransientUiGuard 置位）内重放的 focus/hover 不得弹出摘要提示。
 */
describe('ReleaseItem.vue — 窗口隐藏/失焦后的失效期（防止提示诈尸）', () => {
  afterEach(() => {
    isTransientUiInvalidatedMock.mockReturnValue(false)
  })

  function mountTruncatedSummary() {
    const wrapper = mountRelease(createRelease({ ai_summary: '很长的摘要内容'.repeat(50) }))
    const el = wrapper.find('.release-summary-text')
    // jsdom 无布局引擎：手动造「文本被截断」的条件
    Object.defineProperty(el.element, 'scrollHeight', { value: 100, configurable: true })
    Object.defineProperty(el.element, 'clientHeight', { value: 40, configurable: true })
    el.element.getBoundingClientRect = vi.fn().mockReturnValue({ left: 100, bottom: 140 })
    return { wrapper, el }
  }

  it('失效期内重放的 focus 不弹提示（托盘重开的主复现路径）', async () => {
    const { wrapper, el } = mountTruncatedSummary()
    isTransientUiInvalidatedMock.mockReturnValue(true)

    await el.trigger('focus')

    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(false)
  })

  it('失效期内 hover 也不弹提示', async () => {
    const { wrapper, el } = mountTruncatedSummary()
    isTransientUiInvalidatedMock.mockReturnValue(true)

    await el.trigger('mouseenter', { clientX: 200, clientY: 300 })

    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(false)
  })

  it('失效期结束后（用户真的动了手）focus 恢复弹提示', async () => {
    const { wrapper, el } = mountTruncatedSummary()
    isTransientUiInvalidatedMock.mockReturnValue(true)
    await el.trigger('focus')
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(false)

    isTransientUiInvalidatedMock.mockReturnValue(false)
    await el.trigger('focus')
    expect(wrapper.find('.release-summary-tooltip').exists()).toBe(true)
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

    const snoozeBtn = wrapper.findAll('button').find(b => b.text().includes(t('release.snooze')))
    await snoozeBtn!.trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(t('release.snooze_scheduled'))
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

    expect(toast).toHaveBeenCalledWith(t('release.notification_cancelled'))
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
    }), { [ShowImportanceKey as symbol]: ref(true) })

    expect(wrapper.find('.release-importance-chip').exists()).toBe(true)
    // 组件将中文枚举映射为 i18n key（mock 的 t 原样返回 key）
    expect(wrapper.find('.release-importance-chip').text()).toBe(t('release.importance_high'))
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

  it('「显示重要度」关闭时不显示重要性标签，卡片也不带重要度配色 class', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '修复bug', ai_importance: '大' }), {
      [ShowImportanceKey as symbol]: ref(false),
    })

    expect(wrapper.find('.release-importance-chip').exists()).toBe(false)
    expect(wrapper.find('.release-item').classes()).not.toContain('release-importance-high')
  })
})

describe('ReleaseItem.vue — 操作按钮状态', () => {
  it('操作中(isUpdating)时所有按钮 disabled', async () => {
    vi.mocked(setNotificationState).mockReturnValue(new Promise(() => {})) // 永不 resolve

    const wrapper = mountRelease(createRelease({ notification_status: 'pending' }))

    // 点击 ignore 触发 isUpdating
    wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

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

    expect(wrapper.text()).toContain(t('release.ignore'))
  })

  it('snoozed 状态且 snooze_until 未到期时仍显示 Ignore 按钮（闭环不中断）', () => {
    // snooze_until 未来，按钮判断不传 snooze_until 以保证可取消提醒
    const wrapper = mountRelease(createRelease({
      notification_status: 'snoozed',
      snooze_until: '2099-01-01T00:00:00Z',
    }))

    expect(wrapper.text()).toContain(t('release.ignore'))
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
