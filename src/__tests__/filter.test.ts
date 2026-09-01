import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ref, computed, defineComponent } from 'vue'
import { mount } from '@vue/test-utils'
import { isUnreadStatus, isReadStatus, filterReleaseIndices, buildBodyIndex } from '../utils'

// 模拟 ReleaseTab.vue 的 filteredReleases + sortedReleases 逻辑
// 绕过真实组件依赖（Tauri API、inject 等），直接测试筛选/排序计算

interface MockRelease {
  id: number
  owner: string
  repo: string
  tag_name: string
  release_name: string
  body: string | null
  source_type: string
  published_at: string
  notification_status: string
  snooze_until: string | null
  ai_importance: string | null
}

function makeRelease(id: number, overrides: Partial<MockRelease> = {}): MockRelease {
  return {
    id,
    owner: 'hhelibeb',
    repo: 'relwatch',
    tag_name: `v${id}.0.0`,
    release_name: `Release ${id}.0.0`,
    body: null,
    source_type: 'github', // ← 默认 GitHub：body 归 Tier2，常规搜索不命中
    published_at: `2025-01-${String(id).padStart(2, '0')}T00:00:00Z`,
    notification_status: 'pending',
    snooze_until: null,
    ai_importance: null,
    ...overrides,
  }
}

const FilterTester = defineComponent({
  props: {
    releases: { type: Array as () => MockRelease[], default: () => [] },
    search: { type: String, default: '' },
    statusFilter: { type: String, default: 'all' },
    importanceFilter: { type: String, default: 'all' },
    // 深度搜索索引（与 releases 对齐）；传入即启用 Tier2 搜索
    bodyIndex: { type: Array as () => string[][] | null, default: null },
  },
  emits: ['result'],
  setup(props, { emit }) {
    const releaseSearch = ref(props.search)
    const statusFilter = ref(props.statusFilter)
    const importanceFilter = ref(props.importanceFilter)

    const filteredReleases = computed(() => {
      let list = props.releases as MockRelease[]

      const q = releaseSearch.value.trim()
      if (q) {
        const picked = filterReleaseIndices(list, q, props.bodyIndex)
        list = picked.map(i => list[i])
      }

      if (statusFilter.value === 'unread') {
        list = list.filter(r => isUnreadStatus(r.notification_status, r.snooze_until))
      } else if (statusFilter.value === 'read') {
        list = list.filter(r => isReadStatus(r.notification_status))
      }

      if (importanceFilter.value !== 'all') {
        list = list.filter(r => r.ai_importance === importanceFilter.value)
      }

      return list
    })

    const sortedReleases = computed(() => {
      return [...filteredReleases.value].sort(
        (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime()
      )
    })

    emit('result', sortedReleases.value)

    return { sortedReleases }
  },
  template: '<div />',
})

beforeEach(() => {
  vi.clearAllMocks()
})

describe('ReleaseTab — 筛选/排序逻辑', () => {
  const releases = [
    makeRelease(1, { owner: 'microsoft', repo: 'vscode', notification_status: 'pending', published_at: '2025-03-01T00:00:00Z' }),
    makeRelease(2, { owner: 'microsoft', repo: 'vscode', notification_status: 'clicked', published_at: '2025-02-15T00:00:00Z', tag_name: 'v1.5.0' }),
    makeRelease(3, { owner: 'hhelibeb', repo: 'relwatch', notification_status: 'snoozed', published_at: '2025-03-10T00:00:00Z', ai_importance: '大' }),
    makeRelease(4, { owner: 'hhelibeb', repo: 'relwatch', notification_status: 'ignored', published_at: '2025-01-20T00:00:00Z', ai_importance: '小' }),
    makeRelease(5, { owner: 'tauri-apps', repo: 'tauri', notification_status: 'pending', published_at: '2025-04-01T00:00:00Z', tag_name: 'v2.0.0', body: 'Major release with new features' }),
  ]

  it('无筛选时返回全部 release，按时间降序', () => {
    const wrapper = mount(FilterTester, { props: { releases } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(5)
    // 验证排序：最新的在前
    expect(result[0].id).toBe(5) // 2025-04-01
    expect(result[1].id).toBe(3) // 2025-03-10
    expect(result[2].id).toBe(1) // 2025-03-01
    expect(result[3].id).toBe(2) // 2025-02-15
    expect(result[4].id).toBe(4) // 2025-01-20
  })

  it('搜索关键词过滤', () => {
    const wrapper = mount(FilterTester, { props: { releases, search: 'vscode' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(2)
    expect(result.every(r => r.repo === 'vscode')).toBe(true)
  })

  it('搜索 tag_name', () => {
    // id=2 的 tag_name='v1.5.0', id=5 的 tag_name='v2.0.0'
    // 'v2.0' 只匹配 id=5
    const wrapper = mount(FilterTester, { props: { releases, search: 'v2.0' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(1)
    expect(result[0].id).toBe(5)
  })

  it('GitHub body 默认不命中常规搜索（需深度搜索）', () => {
    const wrapper = mount(FilterTester, { props: { releases, search: 'Major release' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(0)
  })

  it('传入 bodyIndex（深度搜索）后 GitHub body 可命中', () => {
    const wrapper = mount(FilterTester, { props: { releases, search: 'Major release', bodyIndex: buildBodyIndex(releases) } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(1)
    expect(result[0].id).toBe(5)
  })

  it('statusFilter=unread 过滤已读状态', () => {
    const wrapper = mount(FilterTester, { props: { releases, statusFilter: 'unread' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(3) // pending(1,5) + snoozed(3)
    expect(result.every(r => isUnreadStatus(r.notification_status, r.snooze_until))).toBe(true)
  })

  it('statusFilter=unread 排除未到提醒时间的 snoozed', () => {
    const future = new Date(Date.now() + 60_000).toISOString()
    const wrapper = mount(FilterTester, {
      props: {
        releases: [
          makeRelease(1, { notification_status: 'pending' }),
          makeRelease(2, { notification_status: 'snoozed', snooze_until: future }),
        ],
        statusFilter: 'unread',
      },
    })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result.map(r => r.id)).toEqual([1])
  })

  it('statusFilter=read 过滤未读状态', () => {
    const wrapper = mount(FilterTester, { props: { releases, statusFilter: 'read' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(2) // clicked(2) + ignored(4)
    expect(result.every(r => isReadStatus(r.notification_status))).toBe(true)
  })

  it('importanceFilter 过滤', () => {
    const wrapper = mount(FilterTester, { props: { releases, importanceFilter: '大' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(1)
    expect(result[0].ai_importance).toBe('大')
  })

  it('importanceFilter=all 不过滤', () => {
    const wrapper = mount(FilterTester, { props: { releases, importanceFilter: 'all' } })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(5)
  })

  it('组合筛选：搜索 + status + importance', () => {
    const wrapper = mount(FilterTester, {
      props: {
        releases,
        search: 'relwatch',
        statusFilter: 'unread',
        importanceFilter: '大',
      },
    })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(1)
    expect(result[0].id).toBe(3)
  })

  it('组合筛选无结果', () => {
    const wrapper = mount(FilterTester, {
      props: {
        releases,
        search: 'nonexistent',
      },
    })
    const result = wrapper.emitted('result')![0][0] as MockRelease[]
    expect(result).toHaveLength(0)
  })
})
