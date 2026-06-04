import { describe, expect, it, vi } from 'vitest'
import { shallowMount } from '@vue/test-utils'
import ReleaseTab from '../components/ReleaseTab.vue'
import ReleaseSearchBar from '../components/ReleaseSearchBar.vue'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
  getLocale: vi.fn(() => 'en'),
}))

vi.mock('../utils', () => ({
  formatDate: vi.fn(() => '2024-01-01'),
  isReadStatus: vi.fn(() => false),
  isUnreadStatus: vi.fn(() => false),
  releaseMatchesSearch: vi.fn(() => true),
}))

function createRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  const now = new Date()
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'test',
    repo: 'test',
    tag_name: 'v1.0.0',
    release_name: 'Test',
    html_url: 'https://github.com/test/test/releases/tag/v1.0.0',
    published_at: now.toISOString(),
    prerelease: false,
    body: null,
    detected_at: now.toISOString(),
    notification_status: 'pending' as const,
    snooze_until: null,
    ai_summary: null,
    ai_importance: null,
    ...overrides,
  }
}

function createWrapper(releases: ReleaseInfo[] = []) {
  return shallowMount(ReleaseTab, {
    props: { releases },
  })
}

describe('ReleaseTab — ReleaseSearchBar 单实例', () => {
  it('简单视图（默认）：恰好渲染 1 个 ReleaseSearchBar', () => {
    const wrapper = createWrapper()
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })

  it('聚合视图：恰好渲染 1 个 ReleaseSearchBar', async () => {
    const wrapper = createWrapper()
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'aggregated')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })

  it('日历主视图：恰好渲染 1 个 ReleaseSearchBar', async () => {
    const wrapper = createWrapper()
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'calendar')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })

  it('日历钻取视图：恰好渲染 1 个 ReleaseSearchBar（不重复）', async () => {
    const today = new Date()
    const wrapper = createWrapper([createRelease({ published_at: today.toISOString() })])
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'calendar')
    await wrapper.vm.$nextTick()

    // 点一个有内容的日历格子（calendar-cell-count-* 表示 count > 0）
    const cell = wrapper.find('.calendar-cell.current-month.today')
    expect(cell.exists()).toBe(true)
    await cell.trigger('click')
    await wrapper.vm.$nextTick()

    // 确认已切换到钻取视图
    expect(wrapper.find('.date-detail-title').exists()).toBe(true)
    // 确认仍只有 1 个搜索栏（无重复）
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })
})

describe('ReleaseTab — ReleaseSearchBar showSearch', () => {
  it('所有视图下 showSearch prop 始终为 true', async () => {
    const today = new Date()
    const wrapper = createWrapper([createRelease({ published_at: today.toISOString() })])
    const bar = () => wrapper.findComponent(ReleaseSearchBar)

    // 简单视图
    expect((bar().props() as unknown as Record<string, unknown>).showSearch).toBe(true)

    // 聚合视图
    bar().vm.$emit('update:viewMode', 'aggregated')
    await wrapper.vm.$nextTick()
    expect((bar().props() as unknown as Record<string, unknown>).showSearch).toBe(true)

    // 日历主视图
    bar().vm.$emit('update:viewMode', 'calendar')
    await wrapper.vm.$nextTick()
    expect((bar().props() as unknown as Record<string, unknown>).showSearch).toBe(true)

    // 日历钻取视图
    const cell = wrapper.find('.calendar-cell.current-month.today')
    await cell.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.date-detail-title').exists()).toBe(true)
    expect((bar().props() as unknown as Record<string, unknown>).showSearch).toBe(true)
  })
})
