import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import ReleaseTab from '../components/ReleaseTab.vue'
import { ShowImportanceKey } from '../injection-keys'
import type { ReleaseInfo } from '../api/releases'

// ── 后端 mock：目录只带正文预览，全文由分块接口按需取 ─────────────
// 用一个内存"后端"模拟按 id 游标分块（get_release_search_bodies，**从新到旧**）与按
// id 重取（get_release_search_bodies_by_ids），据此断言前端的水位与增量维护行为：
// - 正文命中必须来自分块接口，不能来自目录（目录里是预览投影）；
// - 水位只装得下最近的一段正文，装不下的必须是更早的内容；
// - 目录刷新只补新增行与内容变化行，不整体重建索引。
const backend = vi.hoisted(() => ({
  /** id → 该条的正文 / 译文（模拟 DB 里的全文） */
  bodies: new Map<number, { body: string | null; body_translated: string | null }>(),
  /** 单块最多返回条数（模拟后端按字符预算切分） */
  pageSize: 2,
  chunkCalls: [] as number[],   // 每次分块请求的 beforeId
  byIdsCalls: [] as number[][], // 每次按 id 重取的 id 列表
  detailCalls: [] as number[],  // get_release_detail 调用
}))

vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return {
    ...actual,
    getReleaseSearchBodies: vi.fn(async (beforeId: number) => {
      backend.chunkCalls.push(beforeId)
      const ids = [...backend.bodies.keys()]
        .filter(id => id < beforeId)
        .sort((a, b) => b - a)
      return ids.slice(0, backend.pageSize).map(id => ({ id, ...backend.bodies.get(id)! }))
    }),
    getReleaseSearchBodiesByIds: vi.fn(async (ids: number[]) => {
      backend.byIdsCalls.push([...ids])
      return ids
        .filter(id => backend.bodies.has(id))
        .map(id => ({ id, ...backend.bodies.get(id)! }))
    }),
    getReleaseDetail: vi.fn(async (id: number) => {
      backend.detailCalls.push(id)
      return { id, body: `FULL:${id}`, body_translated: null } as never
    }),
  }
})

// ── 子组件 stub（保留事件与关键 props，便于驱动交互） ────────────

const expandAll = vi.fn()

const SearchBarStub = defineComponent({
  name: 'ReleaseSearchBarStub',
  props: ['count', 'deepSearch', 'deepSearching', 'bodyTruncated', 'importanceFilter'],
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

/** 清空内存后端：每个用例独立，避免正文分块跨用例串数据 */
function resetBackend() {
  backend.bodies.clear()
  backend.chunkCalls.length = 0
  backend.byIdsCalls.length = 0
  backend.detailCalls.length = 0
  backend.pageSize = 2
}

afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
  resetBackend()
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
  /** 等待 rAF 让帧与随后的微任务（jsdom rAF ~16ms） */
  function flushRaf() {
    return new Promise<void>(resolve => setTimeout(resolve, 25))
  }

  /** 目录项：正文只是预览片段（全文只在分块接口里），搜索命中必须来自索引而非目录 */
  function deepRelease(id: number, overrides: Partial<ReleaseInfo> = {}) {
    return createRelease({ id, owner: 'tauri-apps', repo: 'tauri', body: '预览片段', ...overrides })
  }

  it('开启后按 id 游标从新到旧分块预取正文并命中（正文不来自目录）', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()

    // 常规搜索只走 Tier1，目录不含正文 → 无命中，且未发任何取正文请求
    let list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([])
    expect(list.props('hasSearchQuery')).toBe(true)
    expect(list.props('deepSearch')).toBe(false)
    expect(backend.chunkCalls).toEqual([])

    // 开启深度搜索 → 从最大 id 起步取一块 → 命中
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(true)
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])
    // 首次游标须高于任何真实 id；取空后再探一次（beforeId = 已取到的最小 id）确认到库底
    expect(backend.chunkCalls).toEqual([Number.MAX_SAFE_INTEGER, 4])

    // 关闭深度搜索 → 不再用索引，回到常规结果
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', false)
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases').map((r: ReleaseInfo) => r.id)).toEqual([])

    // 重新开启不重拉：已取到的块在会话内驻留
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    expect(backend.chunkCalls).toEqual([Number.MAX_SAFE_INTEGER, 4])
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])
  })

  it('水位优先覆盖最新正文：装不下的必须是更早的内容', async () => {
    backend.pageSize = 1
    backend.bodies.set(4, { body: 'older body keyword-old', body_translated: null })
    backend.bodies.set(6, { body: 'a'.repeat(1_500_000) + ' keyword-new', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4), deepRelease(6)], search: 'keyword-new' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()

    const list = () => wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    // 最新一块就撑满水位 → 游标不再往下，更早的正文没进索引
    expect(backend.chunkCalls).toEqual([Number.MAX_SAFE_INTEGER])
    expect(list().props('releases').map((r: ReleaseInfo) => r.id)).toEqual([6])
    expect(wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).props('bodyTruncated')).toBe(true)

    // 换搜更早那条的关键词 → 搜不到（水位之外），能力边界已显式标注
    await wrapper.setProps({ search: 'keyword-old' } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()
    expect(list().props('releases')).toEqual([])
    expect(backend.byIdsCalls).toEqual([])
  })

  it('目录刷新不整体重建索引：无内容变化时不发取正文请求，命中不丢失', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])

    const chunkCallsBefore = backend.chunkCalls.length
    // 模拟 App.vue：整体替换数组引用，但内容不变（轮询完成 / 标记已读后重拉）
    await wrapper.setProps({ releases: [...releases, deepRelease(4)] } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(true)
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])
    expect(backend.chunkCalls).toHaveLength(chunkCallsBefore)
    expect(backend.byIdsCalls).toEqual([])
  })

  it('新增版本：倒序游标不回补，改按 id 补取', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    const chunkCallsBefore = backend.chunkCalls.length

    // 轮询采到新版本：id 更大，正文接口返回它
    backend.bodies.set(9, { body: 'another Major release note', body_translated: null })
    await wrapper.setProps({ releases: [...releases, deepRelease(4), deepRelease(9)] } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    // 不倒序重走游标（新行在游标起点之上，游标取不到），只按 id 补这一条
    expect(backend.chunkCalls).toHaveLength(chunkCallsBefore)
    expect(backend.byIdsCalls).toEqual([[9]])
    expect(
      wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases')
        .map((r: ReleaseInfo) => r.id).sort((a: number, b: number) => a - b),
    ).toEqual([4, 9])
  })

  it('同帧多次目录刷新只产生一次增量取正文请求（合帧）', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()

    // 同一渲染帧内两次目录刷新（轮询完成与 release-state-changed 各触发一次重拉）
    backend.bodies.set(9, { body: 'Major release nine', body_translated: null })
    await wrapper.setProps({ releases: [...releases, deepRelease(4), deepRelease(9)] } as Parameters<typeof wrapper.setProps>[0])
    await wrapper.setProps({ releases: [...releases, deepRelease(4), deepRelease(9)] } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    expect(backend.byIdsCalls).toEqual([[9]])
  })

  it('译文落库（正文由无到有）→ 按 id 重取变化行，命中随之出现', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()

    // 翻译落库：目录里的 body_translated 由 null 变为非 null（预览），正文接口返回完整译文
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: '完整译文内容' })
    await wrapper.setProps({
      releases: [...releases, deepRelease(4, { body_translated: '完整译文…' })],
      search: '完整译文',
    } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    expect(backend.byIdsCalls).toEqual([[4]])
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('releases').map((r: ReleaseInfo) => r.id)).toEqual([4])
  })

  it('触及字符水位即停止下探，并把能力边界告知搜索栏', async () => {
    // 单条正文即填满水位：取到它之后必须停止，不再向更早的内容下探
    backend.pageSize = 1
    backend.bodies.set(4, { body: 'a'.repeat(1_500_000), body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    const bar = wrapper.findComponent({ name: 'ReleaseSearchBarStub' })
    expect(bar.props('bodyTruncated')).toBe(false)

    await bar.vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()

    expect(backend.chunkCalls).toEqual([Number.MAX_SAFE_INTEGER])
    expect(wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).props('bodyTruncated')).toBe(true)
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

  it('非深度搜索态下 releases 替换不构建 Tier2 索引', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()

    await wrapper.setProps({ releases: [...releases, deepRelease(4)] } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    // 未开启深度搜索 → 不因 releases 变化而偷偷取正文 / 启用 Tier2
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases').map((r: ReleaseInfo) => r.id)).toEqual([])
    expect(backend.chunkCalls).toEqual([])
    expect(backend.byIdsCalls).toEqual([])
  })

  it('深度搜索态下 releases 替换但搜索词已清空 → 不再同步索引', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    const chunkCallsBefore = backend.chunkCalls.length

    // 清空搜索词（watch(releaseSearch) 退出深度搜索态）
    await wrapper.setProps({ search: '' } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()

    // 随后 releases 刷新：deepSearch 已为 false，不应再同步
    backend.bodies.set(9, { body: 'Major release nine', body_translated: null })
    await wrapper.setProps({ releases: [...releases, deepRelease(4), deepRelease(9)] } as Parameters<typeof wrapper.setProps>[0])
    await flushRaf()
    await nextTick()

    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases')).toHaveLength(5)
    expect(backend.chunkCalls).toHaveLength(chunkCallsBefore)
  })

  it('搜索词清空后自动退出深度搜索态（索引在会话内驻留不释放）', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).props('deepSearch')).toBe(true)

    // 清空搜索词（受控组件：search 由父 props 驱动）→ deepSearch 复位、恢复全量列表
    await wrapper.setProps({ search: '' } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()
    const list = wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
    expect(list.props('hasSearchQuery')).toBe(false)
    expect(list.props('deepSearch')).toBe(false)
    expect(list.props('releases')).toHaveLength(4)
  })

  it('数据刷新后在同步前卸载组件 → 取消排队同步，不再发取正文请求', async () => {
    backend.bodies.set(4, { body: 'Major release with new features', body_translated: null })
    const wrapper = mount(ReleaseTab, {
      props: { releases: [...releases, deepRelease(4)], search: 'Major release' },
      global: { stubs },
    })
    await nextTick()
    await wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:deepSearch', true)
    await flushRaf()
    await nextTick()
    const chunkCallsBefore = backend.chunkCalls.length

    // 触发目录刷新 → watch 排队一次 rAF 同步（尚未执行），随即卸载组件（切 tab / 路由离开）
    backend.bodies.set(9, { body: 'Major release nine', body_translated: null })
    await wrapper.setProps({ releases: [...releases, deepRelease(4), deepRelease(9)] } as Parameters<typeof wrapper.setProps>[0])
    wrapper.unmount()
    await flushRaf()
    await nextTick()

    // 卸载后不应再发任何取正文请求
    expect(backend.chunkCalls).toHaveLength(chunkCallsBefore)
    expect(backend.byIdsCalls).toEqual([])
  })
})

// ── 详情弹窗全文（目录里只有预览投影）───────────────────────────

describe('ReleaseTab 详情弹窗取全文', () => {
  /** 目录项（正文为 600 字预览） */
  const withPreview = releases.map(r => (r.id === 2 ? { ...r, body: '预览前 600 字' } : r))

  /** 等一次宏任务：让 getReleaseDetail 的 await 链彻底结算 */
  function flushAsync() {
    return new Promise<void>(resolve => setTimeout(resolve, 0))
  }

  async function openSecond(wrapper: ReturnType<typeof mountTab>) {
    await wrapper.findComponent({ name: 'ReleaseSimpleListStub' })
      .vm.$emit('openDetail', withPreview[1], [withPreview[1]])
    await flushAsync()
    await nextTick()
  }

  it('打开详情 → 调 getReleaseDetail → 内容替换为全文', async () => {
    const wrapper = mount(ReleaseTab, { props: { releases: withPreview }, global: { stubs } })
    await nextTick()

    await openSecond(wrapper)

    expect(backend.detailCalls).toEqual([2])
    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })
    expect(modal.props('release').id).toBe(2)
    expect(modal.props('release').body).toBe('FULL:2')
  })

  it('目录刷新时对打开中的条目重取全文（翻译完成后同步）', async () => {
    const wrapper = mount(ReleaseTab, { props: { releases: withPreview }, global: { stubs } })
    await nextTick()
    await openSecond(wrapper)
    expect(backend.detailCalls).toEqual([2])

    await wrapper.setProps({ releases: [...withPreview] } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()
    expect(backend.detailCalls).toEqual([2, 2])
  })

  it('取全文失败不打断阅读：保留目录里的预览', async () => {
    const { getReleaseDetail } = await import('../api/releases')
    vi.mocked(getReleaseDetail).mockRejectedValueOnce(new Error('err.db_connect'))
    const wrapper = mount(ReleaseTab, { props: { releases: withPreview }, global: { stubs } })
    await nextTick()

    await openSecond(wrapper)

    const modal = wrapper.findComponent({ name: 'ReleaseDetailModalStub' })
    expect(modal.exists()).toBe(true)
    expect(modal.props('release').body).toBe('预览前 600 字')
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
