import { describe, expect, it, vi } from 'vitest'
import { shallowMount, type VueWrapper } from '@vue/test-utils'
import ReleaseTab from '../components/ReleaseTab.vue'
import ReleaseSearchBar from '../components/ReleaseSearchBar.vue'
import ReleaseItem from '../components/ReleaseItem.vue'
import ReleaseSimpleList from '../components/ReleaseSimpleList.vue'
import ReleaseAggregatedList from '../components/ReleaseAggregatedList.vue'
import ReleaseCalendar from '../components/ReleaseCalendar.vue'
import ReleaseDateDetail from '../components/ReleaseDateDetail.vue'
import VirtualList from '../components/common/VirtualList.vue'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
  getLocale: vi.fn(() => 'en'),
}))

vi.mock('../utils', () => ({
  formatDate: vi.fn(() => '2024-01-01'),
  isReadStatus: vi.fn((status: string) => status === 'clicked' || status === 'ignored'),
  isUnreadStatus: vi.fn((status: string) => status === 'pending' || status === 'snoozed'),
  releaseMatchesSearch: vi.fn((release: ReleaseInfo, query: string) => {
    const q = query.trim().toLowerCase()
    if (!q) return true
    return `${release.owner}/${release.repo}`.toLowerCase().includes(q) ||
      release.owner.toLowerCase().includes(q) ||
      release.repo.toLowerCase().includes(q) ||
      release.tag_name.toLowerCase().includes(q) ||
      release.release_name.toLowerCase().includes(q) ||
      (release.body || '').toLowerCase().includes(q)
  }),
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
    body_translated: null,
    extra_metadata: null,
    source_description: null,
    ...overrides,
  }
}

type ReleaseTabTestProps = {
  releases: ReleaseInfo[]
  search?: string
  statusFilter?: 'all' | 'unread' | 'read'
}

function releaseProp(item: VueWrapper): ReleaseInfo {
  return (item.props() as { release: ReleaseInfo }).release
}

async function setReleaseTabProps(wrapper: ReturnType<typeof createWrapper>, props: Partial<ReleaseTabTestProps>) {
  await wrapper.setProps(props as Parameters<typeof wrapper.setProps>[0])
}

function createWrapper(releases: ReleaseInfo[] = [], props: Partial<ReleaseTabTestProps> = {}) {
  return shallowMount(ReleaseTab, {
    props: { releases, ...props },
    global: {
      stubs: {
        ReleaseSimpleList: false,
        ReleaseAggregatedList: false,
        ReleaseCalendar: false,
        ReleaseDateDetail: false,
        // 简单视图经 VirtualList 渲染 ReleaseItem，需真实渲染才能统计到子组件
        VirtualList: false,
      },
    },
  })
}

function createProtectiveReleases(): ReleaseInfo[] {
  return [
    createRelease({
      id: 1,
      owner: 'tauri-apps',
      repo: 'tauri',
      tag_name: 'v2.0.0',
      release_name: 'Tauri stable',
      published_at: '2025-05-03T00:00:00Z',
      notification_status: 'pending',
      ai_importance: '大',
      body: 'desktop framework',
    }),
    createRelease({
      id: 2,
      owner: 'vuejs',
      repo: 'core',
      tag_name: 'v3.5.0',
      release_name: 'Vue minor',
      published_at: '2025-05-02T00:00:00Z',
      notification_status: 'clicked',
      ai_importance: '中',
      body: 'reactivity improvements',
    }),
    createRelease({
      id: 3,
      owner: 'hhelibeb',
      repo: 'relwatch',
      tag_name: 'v1.3.0',
      release_name: 'RelWatch patch',
      published_at: '2025-05-01T00:00:00Z',
      notification_status: 'ignored',
      ai_importance: '小',
      body: 'settings polish',
    }),
    createRelease({
      id: 4,
      owner: 'tauri-apps',
      repo: 'tauri',
      tag_name: 'v2.1.0',
      release_name: 'Tauri patch',
      published_at: '2025-05-04T00:00:00Z',
      notification_status: 'snoozed',
      ai_importance: '中',
      body: 'security fixes',
    }),
  ]
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

describe('ReleaseTab — 保护性行为测试', () => {
  it('搜索过滤后列表变化，并向外同步搜索词', async () => {
    const wrapper = createWrapper(createProtectiveReleases(), { search: 'tauri' })
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(2)

    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:modelValue', 'vue')
    expect(wrapper.emitted('update:search')?.[0]).toEqual(['vue'])

    await setReleaseTabProps(wrapper, { search: 'vue' })
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(1)
    expect(releaseProp(wrapper.findComponent(ReleaseItem))).toMatchObject({ id: 2 })
  })

  it('状态过滤覆盖全部 / 未读 / 已读', async () => {
    const wrapper = createWrapper(createProtectiveReleases())
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(4)

    await setReleaseTabProps(wrapper, { statusFilter: 'unread' })
    expect(wrapper.findAllComponents(ReleaseItem).map(item => releaseProp(item).id)).toEqual([4, 1])

    await setReleaseTabProps(wrapper, { statusFilter: 'read' })
    expect(wrapper.findAllComponents(ReleaseItem).map(item => releaseProp(item).id)).toEqual([2, 3])

    await setReleaseTabProps(wrapper, { statusFilter: 'all' })
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(4)

    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:statusFilter', 'unread')
    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['unread'])
  })

  it('重要度过滤覆盖全部 / 大 / 中 / 小', async () => {
    const wrapper = createWrapper(createProtectiveReleases())
    const bar = wrapper.findComponent(ReleaseSearchBar)

    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(4)

    bar.vm.$emit('update:importanceFilter', '大')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseItem).map(item => releaseProp(item).id)).toEqual([1])

    bar.vm.$emit('update:importanceFilter', '中')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseItem).map(item => releaseProp(item).id)).toEqual([4, 2])

    bar.vm.$emit('update:importanceFilter', '小')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseItem).map(item => releaseProp(item).id)).toEqual([3])

    bar.vm.$emit('update:importanceFilter', 'all')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(4)
  })

  it('可以在简单 / 聚合 / 日历三种视图间切换', async () => {
    const wrapper = createWrapper(createProtectiveReleases())
    const bar = wrapper.findComponent(ReleaseSearchBar)

    expect(wrapper.find('.release-list').exists()).toBe(true)
    expect(wrapper.find('.repo-group').exists()).toBe(false)
    expect(wrapper.find('.calendar-grid').exists()).toBe(false)

    bar.vm.$emit('update:viewMode', 'aggregated')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.repo-group').exists()).toBe(true)
    expect(wrapper.find('.calendar-grid').exists()).toBe(false)

    bar.vm.$emit('update:viewMode', 'calendar')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.calendar-grid').exists()).toBe(true)
    expect(wrapper.find('.repo-group').exists()).toBe(false)

    bar.vm.$emit('update:viewMode', 'simple')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.release-list').exists()).toBe(true)
  })

  it('聚合视图支持展开 / 收起 repo 分组', async () => {
    const wrapper = createWrapper(createProtectiveReleases())
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'aggregated')
    await wrapper.vm.$nextTick()

    const header = wrapper.find('.repo-group-header')
    expect(header.exists()).toBe(true)
    expect(wrapper.find('.repo-group-body').exists()).toBe(false)

    await header.trigger('click')
    expect(wrapper.find('.repo-group-body').exists()).toBe(true)
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(2)

    await header.trigger('click')
    expect(wrapper.find('.repo-group-body').exists()).toBe(false)
  })

  it('日历点击某天进入详情列表', async () => {
    const today = new Date()
    const wrapper = createWrapper([
      createRelease({ id: 10, published_at: today.toISOString(), tag_name: 'v-today' }),
    ])
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'calendar')
    await wrapper.vm.$nextTick()

    await wrapper.get('.calendar-cell.current-month.today').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('.date-detail-title').exists()).toBe(true)
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(1)
    expect(releaseProp(wrapper.findComponent(ReleaseItem))).toMatchObject({ id: 10 })
  })

  it('ReleaseItem 触发 update 后，ReleaseTab 正确向外 emit update', async () => {
    const wrapper = createWrapper(createProtectiveReleases())
    wrapper.findComponent(ReleaseItem).vm.$emit('update')
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('update')).toEqual([[]])
  })
})
