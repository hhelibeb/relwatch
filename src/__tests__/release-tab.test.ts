import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import ReleaseTab from '../components/ReleaseTab.vue'
import { ShowImportanceKey } from '../injection-keys'
import type { ReleaseInfo } from '../api/releases'

// 统计 buildBodyIndex 真实调用次数（转发原实现，不改行为）：
// 锁住「深度搜索索引单飞重建」——同帧多次整体替换应只重建一次，
// 防止将来有人删掉 `if (bodyIndexRebuildRaf !== 0) return` 而测试仍绿。
const buildBodyIndexSpy = vi.hoisted(() => ({ calls: 0 }))
vi.mock('../utils', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../utils')>()
  return {
    ...actual,
    buildBodyIndex: (releases: Parameters<typeof actual.buildBodyIndex>[0]) => {
      buildBodyIndexSpy.calls++
      return actual.buildBodyIndex(releases)
    },
  }
})

// ── 子组件 stub（保留事件与关键 props，便于驱动交互） ────────────

const expandAll = vi.fn()

const SearchBarStub = defineComponent({
  name: 'ReleaseSearchBarStub',
  props: ['count', 'deepSearch', 'deepSearching', 'importanceFilter'],
  emits: ['update:modelValue', 'update:statusFilter', 'update:importanceFilter', 'update:viewMode', 'update:deepSearch', 'update:flagFilter', 'update:versionFilter', 'searchEnter'],
  template: '<div class="toolbar-stub" />',
})

const SimpleListStub = defineComponent({
  name: 'ReleaseSimpleListStub',
  props: ['releases', 'isFiltering', 'hasSearchQuery', 'deepSearch'],
  emits: ['update', 'openDetail', 'enableDeep'],
  template: '<div class="simple-stub" />',
})

const AggregatedListStub = defineComponent({
  name: 'ReleaseAggregatedListStub',
  props: ['releases', 'isFiltering'],
  emits: ['update', 'openDetail'],
  methods: { expandAll },
  template: '<div class="agg-stub" />',
})

const CalendarStub = defineComponent({
  name: 'ReleaseCalendarStub',
  props: ['releases', 'year', 'month'],
  emits: ['prevMonth', 'nextMonth', 'selectDate'],
  template: '<div class="cal-stub" />',
})

const DateDetailStub = defineComponent({
  name: 'ReleaseDateDetailStub',
  props: ['selectedDate', 'releases'],
  emits: ['back', 'update', 'openDetail'],
  template: '<div class="date-detail-stub" />',
})

const DetailModalStub = defineComponent({
  name: 'ReleaseDetailModalStub',
  props: ['release', 'position', 'total', 'hasPrev', 'hasNext'],
  emits: ['close', 'navigate', 'update'],
  template: '<div class="modal-stub" />',
})

const stubs = {
  ReleaseSearchBar: SearchBarStub,
  ReleaseSimpleList: SimpleListStub,
  ReleaseAggregatedList: AggregatedListStub,
  ReleaseCalendar: CalendarStub,
  ReleaseDateDetail: DateDetailStub,
  ReleaseDetailModal: DetailModalStub,
}

// ── fixtures ─────────────────────────────────────────────────────

function createRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'vuejs',
    repo: 'core',
    tag_name: 'v1.0.0',
    release_name: 'v1.0.0',
    html_url: 'https://github.com/vuejs/core/releases/tag/v1.0.0',
    published_at: '2025-01-01T00:00:00Z',
    prerelease: false,
    body: null,
    detected_at: '2025-01-01T00:00:00Z',
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

const releases = [
  createRelease({ id: 1, owner: 'vuejs', repo: 'core', tag_name: 'v3.0.0', notification_status: 'pending' }),
  createRelease({ id: 2, owner: 'microsoft', repo: 'vscode', tag_name: 'v1.90.0', notification_status: 'clicked', ai_importance: '大' }),
  createRelease({ id: 3, owner: 'tauri-apps', repo: 'tauri', tag_name: 'v2.0.0', notification_status: 'pending', ai_importance: '中' }),
]

function mountTab(props: Record<string, unknown> = {}) {
  return mount(ReleaseTab, {
    props: { releases, ...props },
    global: { stubs },
  })
}

function setSystemTime(iso: string) {
  vi.useFakeTimers()
  vi.setSystemTime(new Date(iso))
}

afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
  buildBodyIndexSpy.calls = 0
})

// ── 视图渲染与过滤 ───────────────────────────────────────────────

describe('ReleaseTab 渲染与过滤', () => {
  it('默认 simple 视图，透传全部 releases', () => {
    const wrapper = mountTab()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.exists()).toBe(true)
    expect(list.props('releases')).toHaveLength(3)
  })

  it('搜索过滤：匹配 owner/repo/tag/release_name/body', async () => {
    const wrapper = mountTab({ search: 'vscode' })
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([2])
  })

  it('状态过滤 unread 只保留 pending/snoozed', async () => {
    const wrapper = mountTab({ statusFilter: 'unread' })
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([1, 3])
  })

  it('状态过滤 read 只保留 clicked/ignored', async () => {
    const wrapper = mountTab({ statusFilter: 'read' })
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([2])
  })

  it('重要度过滤 + 搜索叠加生效，isFiltering 随条件变化', async () => {
    const wrapper = mountTab({ search: 'v', statusFilter: 'read' })
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    // vscode 含 v 且 clicked → 只剩 id 2
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([2])
    expect(list.props('isFiltering')).toBe(true)
  })

  it('无过滤时 isFiltering 为 false', () => {
    const wrapper = mountTab()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('isFiltering')).toBe(false)
  })

  it('将 filteredReleases.length 作为 count 传给搜索栏（随筛选变化）', async () => {
    const wrapper = mountTab()
    const bar = wrapper.findComponent({ name: 'ReleaseSearchBarStub' })
    expect(bar.props('count')).toBe(3)

    bar.vm.$emit('update:statusFilter', 'read')
    await nextTick()
    await wrapper.setProps({ statusFilter: 'read' } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()
    expect(bar.props('count')).toBe(1)
  })

  it('切换 aggregated 视图渲染聚合列表', async () => {
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'aggregated')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).exists()).toBe(false)
    expect(wrapper.findComponent({ name: 'ReleaseAggregatedListStub' }).exists()).toBe(true)
  })
})

// ── 深度搜索（Tier2：GitHub / HF 正文与译文全文）─────────────────

describe('ReleaseTab 深度搜索', () => {
  /** 等待 runDeepSearch 内部的 requestAnimationFrame 让帧（jsdom rAF ~16ms） */
  function flushRaf() {
    return new Promise<void>(resolve => setTimeout(resolve, 25))
  }

  it('开启后 GitHub body 可命中，关闭后索引释放', async () => {
    const withBody = [
      ...releases,
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
    ]
    const wrapper = mount(ReleaseTab, {
      props: { releases: withBody, search: 'Major release' },
      global: { stubs },
    })
    await nextTick()

    // 常规搜索：GitHub body 不在 Tier1，无命中
    let list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([])
    expect(list.props('hasSearchQuery')).toBe(true)
    expect(list.props('deepSearch')).toBe(false)

    // 开启深度搜索 → 构建 Tier2，body 命中
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(true)
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])

    // 关闭深度搜索 → 释放索引，回到常规结果
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', false)
    await nextTick()
    list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([])
  })

  it('空结果提示触发 enable-deep 一键开启深度搜索', async () => {
    const wrapper = mountTab({ search: 'Major release' })
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('hasSearchQuery')).toBe(true)
    expect(list.props('deepSearch')).toBe(false)

    await list.vm.$emit('enableDeep')
    await flushRaf()
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('deepSearch')).toBe(true)
  })

  it('深度搜索态下 releases 整体替换（轮询/标记已读）后索引重建，body 命中不丢失', async () => {
    const withBody = [
      ...releases,
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
    ]
    const wrapper = mount(ReleaseTab, {
      props: { releases: withBody, search: 'Major release' },
      global: { stubs },
    })
    await nextTick()

    // 开启深度搜索 → body 命中
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])

    // 模拟 App.vue loadReleases()：整体替换数组引用（轮询完成 / release-state-changed 后重拉）
    const refreshed = withBody.map(r => ({ ...r }))
    await wrapper.setProps({ releases: refreshed } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf() // 重建已改为 rAF 单飞（scheduleBodyIndexRebuild）：需让一帧完成异步重建
    await nextTick()

    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(true)          // 仍处于深度搜索态
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])  // 索引已重建，命中仍在
  })

  it('非深度搜索态下 releases 替换不构建 Tier2 索引', async () => {
    const withBody = [
      ...releases,
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
    ]
    const wrapper = mount(ReleaseTab, {
      props: { releases: withBody, search: 'Major release' },
      global: { stubs },
    })
    await nextTick()

    await wrapper.setProps({ releases: withBody.map(r => ({ ...r })) } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()

    // 未开启深度搜索 → 不因 releases 变化而偷偷启用 Tier2
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([])
  })

  it('深度搜索态下 releases 替换但搜索词已清空 → 索引不重建', async () => {
    const withBody = [
      ...releases,
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
    ]
    const wrapper = mount(ReleaseTab, {
      props: { releases: withBody, search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()

    // 清空搜索词（watch(releaseSearch) 退出深度搜索态）
    await wrapper.setProps({ search: '' } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()

    // 随后 releases 刷新：deepSearch 已为 false，不应再构建
    await wrapper.setProps({ releases: withBody.map(r => ({ ...r })) } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()

    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases')).toHaveLength(4)
  })

  it('搜索词清空后自动退出深度搜索态并释放', async () => {
    const wrapper = mountTab({ search: 'Major release' })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('deepSearch')).toBe(true)

    // 清空搜索词（受控组件：search 由父 props 驱动）→ deepSearch 复位、索引释放、恢复全量列表
    await wrapper.setProps({ search: '' } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('hasSearchQuery')).toBe(false)
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases')).toHaveLength(3)
  })

  it('深度搜索态下同帧多次整体替换合并为一次重建（取最新数据）', async () => {
    const withBody = [
      ...releases,
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
    ]
    const wrapper = mount(ReleaseTab, {
      props: { releases: withBody, search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])

    // 同一渲染帧内连续两次整体替换（如轮询 + 标记已读先后各触发一次 loadReleases）：
    // 单飞 rAF 应把两次合并为一次重建，且最终以最后一次数据为准。
    // 第一次：body 命中词被改成无关词（id 4 不应再命中）
    const first = withBody.map((r, i) => ({ ...r, body: i === 3 ? 'unrelated text here' : r.body }))
    await wrapper.setProps({ releases: first } as Parameters<typeof wrapper.setProps>[0])
    // 第二次（rAF 尚未执行，仍在同一帧）：恢复命中词并新增另一条命中 release
    const second = [
      ...first.slice(0, 3),
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
      createRelease({ id: 5, owner: 'tauri-apps', repo: 'tauri', body: 'another Major release note' }),
    ]
    await wrapper.setProps({ releases: second } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    // 只应做一次重建，且命中使用的是第二次（最新）数据：id 4 与 5 均命中
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('releases').map((r: ReleaseInfo) => r.id).sort((a: number, b: number) => a - b)).toEqual([4, 5])
    // 单飞锁定：开启时 1 次 + 同帧两次替换合并为 1 次 = 共 2 次（若删了单飞保护会变成 3 次）
    expect(buildBodyIndexSpy.calls).toBe(2)
  })

  it('深度搜索态下数据刷新后在重建前卸载组件 → 取消排队重建', async () => {
    const withBody = [
      ...releases,
      createRelease({ id: 4, owner: 'tauri-apps', repo: 'tauri', body: 'Major release with new features' }),
    ]
    const wrapper = mount(ReleaseTab, {
      props: { releases: withBody, search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    const afterOpen = buildBodyIndexSpy.calls

    // 触发数据刷新 → watch 排队一次 rAF 重建（尚未执行）
    await wrapper.setProps({ releases: withBody.map(r => ({ ...r })) } as Parameters<typeof wrapper.setProps>[0])
    // 重建回调执行前卸载组件（切 tab / 路由离开）
    wrapper.unmount()
    await flushRaf() // 若 onUnmounted 未取消 rAF，这里会多跑一次 buildBodyIndex
    await nextTick()

    // 卸载后不应再重建：调用次数仍停留在开启深度搜索那次
    expect(buildBodyIndexSpy.calls).toBe(afterOpen)
  })
})

// ── 月份导航边界 ─────────────────────────────────────────────────

describe('ReleaseTab 月份导航', () => {
  function switchToCalendar(wrapper: VueWrapper) {
    return wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'calendar')
  }

  it('prevMonth 普通递减', async () => {
    setSystemTime('2025-06-15T00:00:00Z')
    const wrapper = mountTab()
    await switchToCalendar(wrapper)
    await nextTick()
    const cal = wrapper.findComponent({ name: 'ReleaseCalendarStub' })
    expect(cal.props('month')).toBe(6)

    await cal.vm.$emit('prevMonth')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseCalendarStub' }).props('month')).toBe(5)
    expect(wrapper.findComponent({ name: 'ReleaseCalendarStub' }).props('year')).toBe(2025)
  })

  it('prevMonth 一月跨年', async () => {
    setSystemTime('2025-01-15T00:00:00Z')
    const wrapper = mountTab()
    await switchToCalendar(wrapper)
    await nextTick()

    await wrapper.findComponent({ name: 'ReleaseCalendarStub' }).vm.$emit('prevMonth')
    await nextTick()
    const cal = wrapper.findComponent({ name: 'ReleaseCalendarStub' })
    expect(cal.props('month')).toBe(12)
    expect(cal.props('year')).toBe(2024)
  })

  it('prevMonth 2010-01 下限保护不变', async () => {
    setSystemTime('2010-01-15T00:00:00Z')
    const wrapper = mountTab()
    await switchToCalendar(wrapper)
    await nextTick()
    const cal = wrapper.findComponent({ name: 'ReleaseCalendarStub' })

    await cal.vm.$emit('prevMonth')
    await nextTick()
    expect(cal.props('month')).toBe(1)
    expect(cal.props('year')).toBe(2010)
  })

  it('nextMonth 从过去月份跨年（不越过当前月）', async () => {
    setSystemTime('2025-06-15T00:00:00Z')
    const wrapper = mountTab()
    await switchToCalendar(wrapper)
    await nextTick()
    const cal = wrapper.findComponent({ name: 'ReleaseCalendarStub' })

    // 2025-06 往前翻 6 次 → 2024-12
    for (let i = 0; i < 6; i++) {
      await cal.vm.$emit('prevMonth')
    }
    await nextTick()
    expect(cal.props('month')).toBe(12)
    expect(cal.props('year')).toBe(2024)

    // 2024-12 → 2025-01 跨年（未超过当前 2025-06，允许）
    await cal.vm.$emit('nextMonth')
    await nextTick()
    expect(cal.props('month')).toBe(1)
    expect(cal.props('year')).toBe(2025)
  })

  it('nextMonth 不越过当前月份上限', async () => {
    setSystemTime('2025-06-15T00:00:00Z')
    const wrapper = mountTab()
    await switchToCalendar(wrapper)
    await nextTick()
    const cal = wrapper.findComponent({ name: 'ReleaseCalendarStub' })

    await cal.vm.$emit('nextMonth')
    await nextTick()
    expect(cal.props('month')).toBe(6)
    expect(cal.props('year')).toBe(2025)
  })
})

// ── 视图切换重置选中日期 ─────────────────────────────────────────

describe('ReleaseTab 视图切换', () => {
  it('日历选中日期后切换视图会重置 selectedDate', async () => {
    setSystemTime('2025-06-15T00:00:00Z')
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'calendar')
    await nextTick()

    // 选中日期 → 显示日期详情
    await wrapper.findComponent({ name: 'ReleaseCalendarStub' }).vm.$emit('selectDate', '2025-06-10')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).exists()).toBe(true)

    // 切走再切回 → 重置回日历
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'simple')
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'calendar')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseCalendarStub' }).exists()).toBe(true)
    expect(wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).exists()).toBe(false)
  })

  it('日期详情返回按钮回到日历', async () => {
    setSystemTime('2025-06-15T00:00:00Z')
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'calendar')
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseCalendarStub' }).vm.$emit('selectDate', '2025-06-10')
    await nextTick()

    await wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).vm.$emit('back')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseCalendarStub' }).exists()).toBe(true)
    expect(wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).exists()).toBe(false)
  })
})

// ── 详情弹窗导航序列 ─────────────────────────────────────────────

describe('ReleaseTab 详情弹窗', () => {
  it('打开弹窗时按序列计算 position/has-prev/has-next', async () => {
    const wrapper = mountTab()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })

    await list.vm.$emit('openDetail', releases[1], [releases[0], releases[1], releases[2]])
    await nextTick()

    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })
    expect(modal.exists()).toBe(true)
    expect(modal.props('position')).toBe(2)
    expect(modal.props('total')).toBe(3)
    expect(modal.props('hasPrev')).toBe(true)
    expect(modal.props('hasNext')).toBe(true)
    expect(modal.props('release').id).toBe(2)
  })

  it('序列首项 has-prev=false，末项 has-next=false', async () => {
    const wrapper = mountTab()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    await list.vm.$emit('openDetail', releases[0], [releases[0], releases[1], releases[2]])
    await nextTick()
    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })
    expect(modal.props('hasPrev')).toBe(false)
    expect(modal.props('hasNext')).toBe(true)
  })

  it('navigate 前后切换并更新 position', async () => {
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).vm.$emit(
      'openDetail', releases[0], [releases[0], releases[1], releases[2]],
    )
    await nextTick()
    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })

    await modal.vm.$emit('navigate', 1)
    await nextTick()
    expect(modal.props('position')).toBe(2)
    expect(modal.props('release').id).toBe(2)
    expect(modal.props('hasPrev')).toBe(true)

    await modal.vm.$emit('navigate', -1)
    await nextTick()
    expect(modal.props('position')).toBe(1)
    expect(modal.props('release').id).toBe(1)
  })

  it('navigate 越界（首项前/末项后）保持当前位置', async () => {
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).vm.$emit(
      'openDetail', releases[0], [releases[0], releases[1], releases[2]],
    )
    await nextTick()
    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })

    await modal.vm.$emit('navigate', -1)
    await nextTick()
    expect(modal.props('position')).toBe(1)

    await modal.vm.$emit('navigate', 99)
    await nextTick()
    expect(modal.props('position')).toBe(1)
  })

  it('close 关闭弹窗', async () => {
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).vm.$emit(
      'openDetail', releases[0], [releases[0], releases[1], releases[2]],
    )
    await nextTick()
    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })

    await modal.vm.$emit('close')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseDetailModalStub' }).exists()).toBe(false)
  })

  it('聚合视图 search-enter 触发 expandAll', async () => {
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', 'aggregated')
    await nextTick()

    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('searchEnter')
    await nextTick()
    expect(expandAll).toHaveBeenCalled()
  })

  it('simple 视图 search-enter 不触发 expandAll', async () => {
    const wrapper = mountTab()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('searchEnter')
    await nextTick()
    expect(expandAll).not.toHaveBeenCalled()
  })
})

// ── 旗标与版本类型过滤（flagFilter / versionFilter）──────────────

describe('ReleaseTab 旗标与版本类型过滤', () => {
  const flagReleases = [
    createRelease({ id: 1, flag: 1, version_bump: 'major' }),
    createRelease({ id: 2, flag: 3, version_bump: 'minor' }),
    createRelease({ id: 3, flag: 0, version_bump: 'patch' }),
    createRelease({ id: 4, flag: 0, version_bump: null, prerelease: true }),
  ]

  function mountFlagTab() {
    return mount(ReleaseTab, { props: { releases: flagReleases }, global: { stubs } })
  }

  async function filterBy(wrapper: VueWrapper, event: string, value: unknown): Promise<number[]> {
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit(event, value)
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    return list.props('releases').map((r: ReleaseInfo) => r.id)
  }

  it('flagFilter=flagged 只保留已标记（1-6）', async () => {
    const wrapper = mountFlagTab()
    expect(await filterBy(wrapper, 'update:flagFilter', 'flagged')).toEqual([1, 2])
  })

  it('flagFilter=unflagged 只保留未标记', async () => {
    const wrapper = mountFlagTab()
    expect(await filterBy(wrapper, 'update:flagFilter', 'unflagged')).toEqual([3, 4])
  })

  it('flagFilter=具体颜色只保留该颜色', async () => {
    const wrapper = mountFlagTab()
    expect(await filterBy(wrapper, 'update:flagFilter', 3)).toEqual([2])
    expect(await filterBy(wrapper, 'update:flagFilter', 1)).toEqual([1])
  })

  it('flagFilter=all 恢复全量', async () => {
    const wrapper = mountFlagTab()
    await filterBy(wrapper, 'update:flagFilter', 'flagged')
    expect(await filterBy(wrapper, 'update:flagFilter', 'all')).toEqual([1, 2, 3, 4])
  })

  it('versionFilter 按 version_bump 过滤 major/minor/patch', async () => {
    const wrapper = mountFlagTab()
    expect(await filterBy(wrapper, 'update:versionFilter', 'major')).toEqual([1])
    expect(await filterBy(wrapper, 'update:versionFilter', 'minor')).toEqual([2])
    expect(await filterBy(wrapper, 'update:versionFilter', 'patch')).toEqual([3])
  })

  it('versionFilter=prerelease 按 prerelease 标记过滤（不看 version_bump）', async () => {
    const wrapper = mountFlagTab()
    expect(await filterBy(wrapper, 'update:versionFilter', 'prerelease')).toEqual([4])
  })

  it('旗标与版本类型叠加过滤', async () => {
    const wrapper = mountFlagTab()
    await filterBy(wrapper, 'update:flagFilter', 'flagged')
    expect(await filterBy(wrapper, 'update:versionFilter', 'major')).toEqual([1])
  })

  it('旗标筛选激活时 isFiltering 为 true', async () => {
    const wrapper = mountFlagTab()
    await filterBy(wrapper, 'update:flagFilter', 'unflagged')
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('isFiltering')).toBe(true)
  })
})

// ── 显示重要度开关 ───────────────────────────────────────────────

describe('ReleaseTab 显示重要度开关', () => {
  it('关闭开关时清空已选的重要度筛选（watch 重置）', async () => {
    const importanceVisible = ref(true)
    const wrapper = mount(ReleaseTab, {
      props: { releases },
      global: {
        stubs,
        provide: { [ShowImportanceKey as symbol]: importanceVisible },
      },
    })
    const bar = wrapper.findComponent({ name: 'ReleaseSearchBarStub' })
    await bar.vm.$emit('update:importanceFilter', '大')
    await nextTick()
    expect(bar.props('importanceFilter')).toBe('大')
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('isFiltering')).toBe(true)

    importanceVisible.value = false
    await nextTick()
    // 开关关闭触发 watch 重置：筛选值回写 all，激活态消失
    expect(bar.props('importanceFilter')).toBe('all')
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('isFiltering')).toBe(false)
  })
})

// ── 通知定位消费（P1-1）─────────────────────────────────────────

describe('ReleaseTab 通知定位（focusTarget）', () => {
  it('focusTarget + token 变化时重置全部筛选并切 simple 视图，调用 SimpleList 定位', async () => {
    const focusReleaseId = vi.fn().mockReturnValue(true)
    const wrapper = mountTab({})
    // 模拟 SimpleList stub 暴露 focusReleaseId（真实组件 defineExpose）
    const simple = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    ;(simple.vm as unknown as { focusReleaseId: typeof focusReleaseId }).focusReleaseId = focusReleaseId

    await wrapper.setProps({ focusTarget: 2, focusToken: 1 } as Parameters<typeof wrapper.setProps>[0])
    await new Promise(r => setTimeout(r, 5))
    await nextTick()

    expect(focusReleaseId).toHaveBeenCalledWith(2)
    // 重置：isFiltering 应为 false（所有筛选复位）
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('isFiltering')).toBe(false)
    // 目标存在则上报 focus-consumed
    expect(wrapper.emitted('focus-consumed')).toBeTruthy()
  })

  it('目标不在数据中（已删除）且数据非空时上报 focus-not-found', async () => {
    const wrapper = mountTab({})
    await wrapper.setProps({ focusTarget: 999, focusToken: 1 } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()

    expect(wrapper.emitted('focus-not-found')).toBeTruthy()
    expect(wrapper.emitted('focus-consumed')).toBeFalsy()
  })

  it('SimpleList 首次定位失败但重试成功时上报 focus-consumed', async () => {
    let calls = 0
    const wrapper = mountTab({})
    const simple = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    ;(simple.vm as unknown as { focusReleaseId: () => boolean }).focusReleaseId = () => {
      calls += 1
      return calls > 1 // 首次未挂载 → false，重试时已就绪 → true
    }

    await wrapper.setProps({ focusTarget: 1, focusToken: 1 } as Parameters<typeof wrapper.setProps>[0])
    await new Promise(r => setTimeout(r, 5))
    await nextTick()

    expect(calls).toBe(2)
    expect(wrapper.emitted('focus-consumed')).toBeTruthy()
    expect(wrapper.emitted('focus-not-found')).toBeFalsy()
  })

  it('SimpleList 重试后仍定位失败时上报 focus-not-found（不留静默失败）', async () => {
    const wrapper = mountTab({})
    const simple = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    ;(simple.vm as unknown as { focusReleaseId: () => boolean }).focusReleaseId = () => false

    await wrapper.setProps({ focusTarget: 1, focusToken: 1 } as Parameters<typeof wrapper.setProps>[0])
    await new Promise(r => setTimeout(r, 5))
    await nextTick()

    // 目标确实在数据里，但列表定位不到：必须给出反馈（Toast 由 App 渲染），且只上报一次
    expect(wrapper.emitted('focus-consumed')).toBeFalsy()
    expect(wrapper.emitted('focus-not-found')).toHaveLength(1)

    // 同 token 的后续数据刷新不应重复上报
    await wrapper.setProps({ releases: [...releases] } as Parameters<typeof wrapper.setProps>[0])
    await new Promise(r => setTimeout(r, 5))
    await nextTick()
    expect(wrapper.emitted('focus-not-found')).toHaveLength(1)
  })

  it('releases 数据后续到达（冷启动竞态）后自动消费', async () => {
    const focusReleaseId = vi.fn().mockReturnValue(true)
    const wrapper = mount(ReleaseTab, {
      props: { releases: [], focusTarget: 5, focusToken: 1 },
      global: { stubs },
    })
    // 初始数据为空：不消费也不报 not-found
    await nextTick()
    expect(wrapper.emitted('focus-not-found')).toBeFalsy()

    // 数据到达后 watch(releases) 触发消费
    await wrapper.setProps({ releases: [...releases, createRelease({ id: 5, owner: 'x', repo: 'y' })] } as Parameters<typeof wrapper.setProps>[0])
    const simple = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    ;(simple.vm as unknown as { focusReleaseId: typeof focusReleaseId }).focusReleaseId = focusReleaseId
    await new Promise(r => setTimeout(r, 5))
    await nextTick()

    expect(focusReleaseId).toHaveBeenCalledWith(5)
    expect(wrapper.emitted('focus-consumed')).toBeTruthy()
  })

  it('同一 token 重复数据变化不会重复消费', async () => {
    const focusReleaseId = vi.fn().mockReturnValue(true)
    const wrapper = mountTab({})
    const simple = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    ;(simple.vm as unknown as { focusReleaseId: typeof focusReleaseId }).focusReleaseId = focusReleaseId

    await wrapper.setProps({ focusTarget: 1, focusToken: 1 } as Parameters<typeof wrapper.setProps>[0])
    await new Promise(r => setTimeout(r, 5))
    await nextTick()
    // 再次数据刷新（同 token）不应再次定位
    await wrapper.setProps({ releases: [...releases] } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()

    expect(focusReleaseId).toHaveBeenCalledTimes(1)
    expect(wrapper.emitted('focus-consumed')).toHaveLength(1)
  })
})
