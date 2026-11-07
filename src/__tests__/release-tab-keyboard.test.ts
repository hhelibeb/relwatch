import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { enableAutoUnmount, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import ReleaseTab from '../components/ReleaseTab.vue'
import type { ReleaseInfo } from '../api/releases'

// ReleaseTab 的键盘导航、视图状态持久化与日历跳转（子组件全部 stub，只验编排）。
// tab 用 v-show 切换、组件常驻挂载，故快捷键必须靠 active 判定归属，
// 且持久化的读写要跨挂载验证。

vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return {
    ...actual,
    getReleaseSearchBodies: vi.fn(async () => []),
    getReleaseSearchBodiesByIds: vi.fn(async () => []),
    getReleaseDetail: vi.fn(async (id: number) => ({ id, body: null, body_translated: null })),
  }
})
vi.mock('../composables/useUsageTracking', () => ({ track: vi.fn() }))

const focusInput = vi.fn()
const moveSelection = vi.fn()
const selectFirst = vi.fn(() => true)
const openSelected = vi.fn()
const expandAll = vi.fn()

const SearchBarStub = defineComponent({
  name: 'ReleaseSearchBarStub',
  props: ['statusFilter', 'importanceFilter', 'sourceFilter', 'viewMode', 'flagFilter', 'versionFilter'],
  emits: ['update:modelValue', 'update:statusFilter', 'update:importanceFilter', 'update:viewMode',
    'update:deepSearch', 'update:flagFilter', 'update:versionFilter', 'searchEnter'],
  methods: { focusInput },
  template: '<div class="bar-stub" />',
})

const SimpleListStub = defineComponent({
  name: 'ReleaseSimpleListStub',
  props: ['releases', 'isFiltering'],
  emits: ['update', 'openDetail', 'enableDeep', 'clearSearch', 'clearFilters'],
  methods: { focusReleaseId: () => true, moveSelection, selectFirst, openSelected },
  template: '<div class="simple-stub" />',
})

const AggregatedListStub = defineComponent({
  name: 'ReleaseAggregatedListStub',
  props: ['releases'],
  emits: ['update', 'openDetail', 'clearSearch', 'clearFilters'],
  methods: { expandAll },
  template: '<div class="agg-stub" />',
})

const CalendarStub = defineComponent({
  name: 'ReleaseCalendarStub',
  props: ['releases', 'year', 'month'],
  emits: ['prevMonth', 'nextMonth', 'jumpTo', 'selectDate'],
  template: '<div class="cal-stub" />',
})

const DateDetailStub = defineComponent({
  name: 'ReleaseDateDetailStub',
  props: ['selectedDate', 'releases'],
  emits: ['back', 'update', 'openDetail'],
  template: '<div class="date-stub" />',
})

const DetailModalStub = defineComponent({
  name: 'ReleaseDetailModalStub',
  props: ['release'],
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

function createRelease(id: number): ReleaseInfo {
  return {
    id,
    source_id: 1,
    source_type: 'github',
    owner: 'owner',
    repo: `repo${id}`,
    tag_name: `v${id}`,
    release_name: `v${id}`,
    html_url: `https://example.com/${id}`,
    published_at: `2025-0${id}-01T00:00:00Z`,
    prerelease: false,
    body: id === 1 ? '正文' : null,
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
  }
}

const releases = [createRelease(1), createRelease(2), createRelease(3)]

function mountTab(props: Record<string, unknown> = {}): VueWrapper {
  return mount(ReleaseTab, { props: { releases, ...props }, global: { stubs } }) as unknown as VueWrapper
}

function pressKey(key: string, opts: { target?: HTMLElement; ctrlKey?: boolean; isComposing?: boolean } = {}) {
  const event = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
    ctrlKey: opts.ctrlKey ?? false,
    isComposing: opts.isComposing ?? false,
  })
  ;(opts.target ?? document.body).dispatchEvent(event)
  return event
}

function appendEl(tag: string, attrs: Record<string, string> = {}): HTMLElement {
  const el = document.createElement(tag)
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v)
  document.body.appendChild(el)
  return el
}

async function setViewMode(wrapper: VueWrapper, mode: string) {
  wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('update:viewMode', mode)
  await nextTick()
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
})

afterEach(() => {
  vi.useRealTimers()
  document.body.innerHTML = ''
})

enableAutoUnmount(afterEach)

describe('ReleaseTab.vue — j/k 列表导航', () => {
  it('active 时 j/k 分别向下/向上移动光标并拦截默认行为', () => {
    const wrapper = mountTab({ active: true })

    expect(pressKey('j').defaultPrevented).toBe(true)
    expect(pressKey('k').defaultPrevented).toBe(true)
    expect(moveSelection.mock.calls.map(c => c[0])).toEqual([1, -1])
  })

  it('tab 隐藏（active=false）时不响应：v-show 下组件常驻挂载，否则会抢键盘', () => {
    mountTab()

    pressKey('j')
    expect(moveSelection).not.toHaveBeenCalled()
  })

  it('非简单视图不响应 j/k', async () => {
    const wrapper = mountTab({ active: true })
    await setViewMode(wrapper, 'aggregated')

    pressKey('j')
    expect(moveSelection).not.toHaveBeenCalled()
  })

  it('详情弹窗打开时不响应（键盘归弹窗）', async () => {
    const wrapper = mountTab({ active: true })
    wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).vm.$emit('openDetail', releases[0], releases)
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseDetailModalStub' }).exists()).toBe(true)

    pressKey('j')
    expect(moveSelection).not.toHaveBeenCalled()
  })

  it.each([
    ['input', {}],
    ['textarea', {}],
    ['select', {}],
    ['div', { contenteditable: 'true' }],
  ])('%s 内的 j 不参与列表导航', (tag, attrs) => {
    mountTab({ active: true })
    pressKey('j', { target: appendEl(tag, attrs) })

    expect(moveSelection).not.toHaveBeenCalled()
  })

  it('Enter 打开选中项；焦点在按钮上时让位给原生点击', () => {
    mountTab({ active: true })

    pressKey('Enter')
    expect(openSelected).toHaveBeenCalledTimes(1)

    pressKey('Enter', { target: appendEl('button') })
    expect(openSelected).toHaveBeenCalledTimes(1)
  })

  it('带 Ctrl 或输入法合成中的 j 不响应', () => {
    mountTab({ active: true })

    pressKey('j', { ctrlKey: true })
    pressKey('j', { isComposing: true })
    expect(moveSelection).not.toHaveBeenCalled()
  })

  it('搜索框 Enter 在简单视图选中首条并交出焦点', async () => {
    const wrapper = mountTab({ active: true })
    const input = appendEl('input')
    input.focus()
    expect(document.activeElement).toBe(input)

    wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('searchEnter')
    await nextTick()
    expect(selectFirst).toHaveBeenCalledTimes(1)
    expect(document.activeElement).not.toBe(input)
  })

  it('搜索框 Enter 在聚合视图展开全部分组', async () => {
    const wrapper = mountTab({ active: true })
    await setViewMode(wrapper, 'aggregated')

    wrapper.findComponent({ name: 'ReleaseSearchBarStub' }).vm.$emit('searchEnter')
    await nextTick()
    expect(expandAll).toHaveBeenCalledTimes(1)
    expect(selectFirst).not.toHaveBeenCalled()
  })

  it('focusSearch 先关掉遮住搜索栏的详情弹窗，再交给搜索栏 focusInput', async () => {
    const wrapper = mountTab()
    wrapper.findComponent({ name: 'ReleaseSimpleListStub' }).vm.$emit('openDetail', releases[0], releases)
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseDetailModalStub' }).exists()).toBe(true)

    ;(wrapper.vm as unknown as { focusSearch: () => void }).focusSearch()
    await nextTick()
    expect(focusInput).toHaveBeenCalledTimes(1)
    expect(wrapper.findComponent({ name: 'ReleaseDetailModalStub' }).exists()).toBe(false)
  })
})

describe('ReleaseTab.vue — 视图状态持久化', () => {
  it('视图与筛选变更后重新挂载即恢复', async () => {
    const wrapper = mountTab()
    const bar = wrapper.findComponent({ name: 'ReleaseSearchBarStub' })
    bar.vm.$emit('update:viewMode', 'aggregated')
    bar.vm.$emit('update:importanceFilter', '大')
    bar.vm.$emit('update:versionFilter', 'major')
    await nextTick()
    wrapper.unmount()

    const restored = mountTab()
    expect(restored.findComponent({ name: 'ReleaseAggregatedListStub' }).exists()).toBe(true)
    expect(restored.findComponent({ name: 'ReleaseSearchBarStub' }).props('importanceFilter')).toBe('大')
    expect(restored.findComponent({ name: 'ReleaseSearchBarStub' }).props('versionFilter')).toBe('major')
  })

  it('宿主传入的 statusFilter 变化同样落盘', async () => {
    const wrapper = mountTab({ statusFilter: 'all' })
    await wrapper.setProps({ statusFilter: 'unread' })
    await nextTick()
    wrapper.unmount()

    expect(JSON.parse(localStorage.getItem('relwatch.release.view')!).statusFilter).toBe('unread')
  })

  it('损坏的存储值回退默认视图', () => {
    localStorage.setItem('relwatch.release.view', '{"viewMode":"wat","flagFilter":99}')

    expect(mountTab().findComponent({ name: 'ReleaseSimpleListStub' }).exists()).toBe(true)
  })
})

describe('ReleaseTab.vue — 日历快速跳转', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 2, 15, 12))
  })

  it('jumpTo 切到目标年月（从日期详情返回后仍生效）', async () => {
    const wrapper = mountTab()
    await setViewMode(wrapper, 'calendar')
    wrapper.findComponent({ name: 'ReleaseCalendarStub' }).vm.$emit('selectDate', '2026-03-02')
    await nextTick()
    expect(wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).exists()).toBe(true)

    wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).vm.$emit('back')
    await nextTick()
    wrapper.findComponent({ name: 'ReleaseCalendarStub' }).vm.$emit('jumpTo', 2024, 8)
    await nextTick()
    const cal = wrapper.findComponent({ name: 'ReleaseCalendarStub' })
    expect(cal.props('year')).toBe(2024)
    expect(cal.props('month')).toBe(8)
    expect(wrapper.findComponent({ name: 'ReleaseDateDetailStub' }).exists()).toBe(false)
  })

  it('jumpTo 越界时钳到边界（不早于 2010-01，不晚于当前月）', async () => {
    const wrapper = mountTab()
    await setViewMode(wrapper, 'calendar')
    const calendar = wrapper.findComponent({ name: 'ReleaseCalendarStub' })

    calendar.vm.$emit('jumpTo', 1999, 7)
    await nextTick()
    expect(calendar.props('year')).toBe(2010)
    expect(calendar.props('month')).toBe(1)

    calendar.vm.$emit('jumpTo', 2030, 1)
    await nextTick()
    expect(calendar.props('year')).toBe(2026)
    expect(calendar.props('month')).toBe(3)
  })

  it('prev/next 跨年翻页与年月跳转共用同一边界口径', async () => {
    const wrapper = mountTab()
    await setViewMode(wrapper, 'calendar')
    const calendar = wrapper.findComponent({ name: 'ReleaseCalendarStub' })
    calendar.vm.$emit('jumpTo', 2010, 1)
    await nextTick()

    calendar.vm.$emit('prevMonth')
    await nextTick()
    expect(calendar.props('year')).toBe(2010)
    expect(calendar.props('month')).toBe(1)

    calendar.vm.$emit('jumpTo', 2010, 12)
    await nextTick()
    calendar.vm.$emit('nextMonth')
    await nextTick()
    expect(calendar.props('year')).toBe(2011)
    expect(calendar.props('month')).toBe(1)
  })
})
