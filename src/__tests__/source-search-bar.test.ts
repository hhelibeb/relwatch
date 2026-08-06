import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, ref } from 'vue'
import SourceTab from '../components/SourceTab.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ message: vi.fn(), confirm: vi.fn() }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
  tm: vi.fn((key: string, _args: Record<string, string>) => key),
  getLocale: vi.fn(() => 'en'),
}))

// ============ Test helpers ============

interface TestSource {
  id: number
  owner: string
  repo: string
  enabled: boolean
  muted: boolean
  description: string | null
  last_checked_at: string | null
  last_check_status: string
  created_at: string
  config: string | null
}

function createSource(overrides: Partial<TestSource> = {}): TestSource {
  return {
    id: 1,
    owner: 'owner-a',
    repo: 'repo-a',
    enabled: true,
    muted: false,
    description: null,
    last_checked_at: '2025-06-01T00:00:00Z',
    last_check_status: 'ok',
    created_at: '2025-06-01T00:00:00Z',
    config: null,
    ...overrides,
  }
}

function createComponentSource(overrides: Record<string, unknown> = {}) {
  return {
    ...createSource(),
    source_type: 'github',
    poll_interval_minutes: 60,
    last_check_message: null,
    consecutive_failures: 0,
    last_new_count: 0,
    created_at: '2025-06-01T00:00:00Z',
    updated_at: '2025-06-01T00:00:00Z',
    config: null,
    ...overrides,
  }
}

type SourceSortField = 'default' | 'name' | 'status' | 'created'
type SortDirection = 'asc' | 'desc'

function sourceMatchesSearch(source: TestSource, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  const name = `${source.owner}/${source.repo}`.toLowerCase()
  return name.includes(q) ||
    source.owner.toLowerCase().includes(q) ||
    source.repo.toLowerCase().includes(q) ||
    (source.description ?? '').toLowerCase().includes(q)
}

function sortSources(sources: TestSource[], field: SourceSortField, direction: SortDirection, unreadCounts: Record<string, number>): TestSource[] {
  const factor = direction === 'asc' ? 1 : -1
  return [...sources].sort((a, b) => {
    let result: number
    if (field === 'default') {
      const keyA = `${a.owner}/${a.repo}`.toLowerCase()
      const keyB = `${b.owner}/${b.repo}`.toLowerCase()
      const aPending = unreadCounts[keyA] || 0
      const bPending = unreadCounts[keyB] || 0
      result = aPending - bPending || a.id - b.id
    } else if (field === 'name') {
      result = `${a.owner}/${a.repo}`.localeCompare(`${b.owner}/${b.repo}`)
    } else if (field === 'status') {
      if (a.enabled !== b.enabled) result = a.enabled ? -1 : 1
      else result = b.id - a.id
    } else {
      result = a.created_at.localeCompare(b.created_at) || a.id - b.id
    }
    return result * factor
  })
}

// ============ sourceMatchesSearch ============

describe('sourceMatchesSearch', () => {
  const src = createSource({ owner: 'vuejs', repo: 'core', description: 'frontend framework' })

  it('empty query matches all', () => {
    expect(sourceMatchesSearch(src, '')).toBe(true)
    expect(sourceMatchesSearch(src, '   ')).toBe(true)
  })

  it('matches owner/repo', () => {
    expect(sourceMatchesSearch(src, 'vuejs/core')).toBe(true)
    expect(sourceMatchesSearch(src, 'vue')).toBe(true)
    expect(sourceMatchesSearch(src, 'CORE')).toBe(true)
  })

  it('matches description', () => {
    expect(sourceMatchesSearch(src, 'frontend')).toBe(true)
    expect(sourceMatchesSearch(src, 'framework')).toBe(true)
  })

  it('does not match when no field contains query', () => {
    expect(sourceMatchesSearch(src, 'nonexistent')).toBe(false)
    expect(sourceMatchesSearch(src, 'react')).toBe(false)
  })
})

// ============ sortSources ============

describe('sortSources', () => {
  const sources = [
    createSource({ id: 1, owner: 'b', repo: 'beta', enabled: true, created_at: '2025-06-01T00:00:00Z' }),
    createSource({ id: 2, owner: 'a', repo: 'alpha', enabled: true, created_at: '2025-06-10T00:00:00Z' }),
    createSource({ id: 3, owner: 'c', repo: 'gamma', enabled: false, created_at: '2025-05-01T00:00:00Z' }),
  ]
  const emptyCounts: Record<string, number> = {}

  it('default field desc: unread count desc, then id desc', () => {
    const highUnread = createSource({ id: 4, owner: 'd', repo: 'delta', enabled: true })
    const allSources = [...sources, highUnread]
    const counts = { 'd/delta': 5, 'b/beta': 0, 'a/alpha': 0, 'c/gamma': 0 }
    const sorted = sortSources(allSources, 'default', 'desc', counts)
    expect(sorted[0].id).toBe(4)
    expect(sorted[1].id).toBe(3)
    expect(sorted[2].id).toBe(2)
    expect(sorted[3].id).toBe(1)
  })

  it('default field asc reverses default ordering', () => {
    const highUnread = createSource({ id: 4, owner: 'd', repo: 'delta', enabled: true })
    const allSources = [...sources, highUnread]
    const counts = { 'd/delta': 5, 'b/beta': 0, 'a/alpha': 0, 'c/gamma': 0 }
    const sorted = sortSources(allSources, 'default', 'asc', counts)
    expect(sorted[0].id).toBe(1)
    expect(sorted[1].id).toBe(2)
    expect(sorted[2].id).toBe(3)
    expect(sorted[3].id).toBe(4)
  })

  it('name field supports ascending and descending', () => {
    const asc = sortSources(sources, 'name', 'asc', emptyCounts)
    expect(asc.map(s => s.owner)).toEqual(['a', 'b', 'c'])

    const desc = sortSources(sources, 'name', 'desc', emptyCounts)
    expect(desc.map(s => s.owner)).toEqual(['c', 'b', 'a'])
  })

  it('status field supports ascending and descending', () => {
    const asc = sortSources(sources, 'status', 'asc', emptyCounts)
    expect(asc.map(s => s.enabled)).toEqual([true, true, false])

    const desc = sortSources(sources, 'status', 'desc', emptyCounts)
    expect(desc.map(s => s.enabled)).toEqual([false, true, true])
  })

  it('created field uses added time instead of last checked time', () => {
    const asc = sortSources(sources, 'created', 'asc', emptyCounts)
    expect(asc.map(s => s.id)).toEqual([3, 1, 2])

    const desc = sortSources(sources, 'created', 'desc', emptyCounts)
    expect(desc.map(s => s.id)).toEqual([2, 1, 3])
  })
})

// ============ Selection & Bulk operations ============

const MinimalSourceWithBulk = defineComponent({
  props: { sources: { type: Array as () => TestSource[], default: () => [] } },
  emits: ['update'],
  setup(props, { emit }) {
    const selectionMode = ref(false)
    const selectedIds = ref<Set<number>>(new Set())
    const bulkBusy = ref(false)

    function toggleSelection(id: number) {
      const next = new Set(selectedIds.value)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      selectedIds.value = next
    }

    function selectAllVisible() {
      selectedIds.value = new Set((props.sources as TestSource[]).map(s => s.id))
    }

    function clearSelectedSources() {
      selectedIds.value = new Set()
    }

    function clearSelection() {
      selectedIds.value = new Set()
      selectionMode.value = false
    }

    async function handleBulkToggle(enabled: boolean) {
      const ids = [...selectedIds.value]
      if (ids.length === 0) return
      bulkBusy.value = true
      for (const id of ids) {
        const { invoke } = await import('@tauri-apps/api/core')
        await (invoke as any)('update_source', { id, enabled })
      }
      bulkBusy.value = false
      emit('update')
    }

    async function handleBulkMuteToggle(muted: boolean) {
      const ids = [...selectedIds.value]
      if (ids.length === 0) return
      bulkBusy.value = true
      for (const id of ids) {
        const { invoke } = await import('@tauri-apps/api/core')
        await (invoke as any)('update_source', { id, muted })
      }
      bulkBusy.value = false
      emit('update')
    }

    async function handleBulkRemove() {
      const ids = [...selectedIds.value]
      if (ids.length === 0) return
      const { confirm } = await import('@tauri-apps/plugin-dialog')
      const ok = await (confirm as any)('delete?', { kind: 'warning' })
      if (!ok) return
      bulkBusy.value = true
      for (const id of ids) {
        const { invoke } = await import('@tauri-apps/api/core')
        await (invoke as any)('remove_source', { id })
      }
      bulkBusy.value = false
      emit('update')
    }

    const selectedCount = () => selectedIds.value.size

    return { selectionMode, selectedIds, bulkBusy, toggleSelection, selectAllVisible, clearSelectedSources, clearSelection, handleBulkToggle, handleBulkMuteToggle, handleBulkRemove, selectedCount }
  },
  template: `
    <div>
      <button class="btn-select" @click="selectionMode = !selectionMode; if (!selectionMode) clearSelection()">
        {{ selectionMode ? '取消选择' : '选择' }}
      </button>
      <div v-if="selectionMode" class="bulk-bar">
        <span class="bulk-count">已选 {{ selectedCount() }} 项</span>
        <button class="btn-sm btn-select-all" @click="selectAllVisible()">全选</button>
        <button class="btn-sm btn-clear-selection" :disabled="selectedCount() === 0" @click="clearSelectedSources()">取消全选</button>
        <button class="btn-sm btn-resume" :disabled="selectedCount() === 0 || bulkBusy" @click="handleBulkToggle(true)">恢复</button>
        <button class="btn-sm btn-pause" :disabled="selectedCount() === 0 || bulkBusy" @click="handleBulkToggle(false)">暂停</button>
        <button class="btn-sm btn-mute" :disabled="selectedCount() === 0 || bulkBusy" @click="handleBulkMuteToggle(true)">静默</button>
        <button class="btn-sm btn-unmute" :disabled="selectedCount() === 0 || bulkBusy" @click="handleBulkMuteToggle(false)">取消静默</button>
        <button class="btn-sm btn-danger btn-delete" :disabled="selectedCount() === 0 || bulkBusy" @click="handleBulkRemove">删除</button>
      </div>
      <div v-for="s in sources" :key="s.id" class="source-item">
        <div v-if="selectionMode" class="source-checkbox">
          <input type="checkbox" :checked="selectedIds.has(s.id)" @change="toggleSelection(s.id)" />
        </div>
        <span class="source-name">{{ s.owner }}/{{ s.repo }}</span>
      </div>
    </div>
  `,
})

afterEach(() => {
  vi.clearAllMocks()
  window.localStorage.clear()
})

describe('selection mode — toggle / cancel', () => {
  it('enters and exits selection mode', async () => {
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources: [createSource({ id: 1 })] } })
    expect(wrapper.find('.source-checkbox').exists()).toBe(false)

    await wrapper.get('.btn-select').trigger('click')
    expect(wrapper.find('.source-checkbox').exists()).toBe(true)
    expect(wrapper.find('.bulk-bar').exists()).toBe(true)

    await wrapper.get('.btn-select').trigger('click')
    expect(wrapper.find('.source-checkbox').exists()).toBe(false)
    expect(wrapper.find('.bulk-bar').exists()).toBe(false)
  })

  it('selects and deselects individual items', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'b', repo: 'two' })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    const checkboxes = wrapper.findAll('.source-checkbox input[type="checkbox"]')
    expect(checkboxes).toHaveLength(2)

    await checkboxes[0].setValue(true)
    expect((wrapper.vm as any).selectedIds.has(1)).toBe(true)
    expect((wrapper.vm as any).selectedIds.has(2)).toBe(false)

    await checkboxes[0].setValue(false)
    expect((wrapper.vm as any).selectedIds.has(1)).toBe(false)
  })

  it('selectAllVisible selects all items', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'b', repo: 'two' })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')

    expect((wrapper.vm as any).selectedIds.size).toBe(2)
  })

  it('clear selection button empties current selection without exiting mode', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'b', repo: 'two' })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    expect((wrapper.vm as any).selectedIds.size).toBe(2)

    await wrapper.get('.btn-clear-selection').trigger('click')
    expect((wrapper.vm as any).selectedIds.size).toBe(0)
    expect(wrapper.find('.bulk-bar').exists()).toBe(true)
    expect(wrapper.find('.source-checkbox').exists()).toBe(true)
  })

  it('cancel clears selection and exits mode', async () => {
    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'b', repo: 'two' })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    expect((wrapper.vm as any).selectedIds.size).toBe(2)

    await wrapper.get('.btn-select').trigger('click')
    expect((wrapper.vm as any).selectedIds.size).toBe(0)
    expect(wrapper.find('.source-checkbox').exists()).toBe(false)
  })
})

describe('bulk operations', () => {
  it('bulk toggle (pause/resume) calls API and emits update', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'b', repo: 'two' })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    await wrapper.get('.btn-pause').trigger('click')
    await flushPromises()

    expect(invoke).toHaveBeenCalledTimes(2)
    expect(invoke).toHaveBeenCalledWith('update_source', { id: 1, enabled: false })
    expect(invoke).toHaveBeenCalledWith('update_source', { id: 2, enabled: false })
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('bulk delete with confirmation calls API', async () => {
    const { confirm } = await import('@tauri-apps/plugin-dialog')
    ;(confirm as ReturnType<typeof vi.fn>).mockResolvedValue(true)
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    const sources = [createSource({ id: 1 })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    await wrapper.get('.btn-delete').trigger('click')
    await flushPromises()

    expect(confirm).toHaveBeenCalled()
    expect(invoke).toHaveBeenCalledWith('remove_source', { id: 1 })
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('bulk delete cancelled does NOT call API', async () => {
    const { confirm } = await import('@tauri-apps/plugin-dialog')
    ;(confirm as ReturnType<typeof vi.fn>).mockResolvedValue(false)
    const { invoke } = await import('@tauri-apps/api/core')

    const sources = [createSource({ id: 1 })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    await wrapper.get('.btn-delete').trigger('click')
    await flushPromises()

    expect(confirm).toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  it('buttons disabled during bulk operation', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockImplementation(() => new Promise(() => {}))

    const sources = [createSource({ id: 1 })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    wrapper.get('.btn-pause').trigger('click')
    await flushPromises()

    expect(wrapper.get('.btn-pause').attributes('disabled')).toBeDefined()
    expect(wrapper.get('.btn-delete').attributes('disabled')).toBeDefined()
    expect(wrapper.get('.btn-resume').attributes('disabled')).toBeDefined()
  })
})

describe('bulk mute/unmute operations', () => {
  it('bulk mute calls API with muted=true', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    const sources = [createSource({ id: 1 }), createSource({ id: 2, owner: 'b', repo: 'two' })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    await (wrapper.vm as any).handleBulkMuteToggle(true)
    await flushPromises()

    expect(invoke).toHaveBeenCalledTimes(2)
    expect(invoke).toHaveBeenCalledWith('update_source', { id: 1, muted: true })
    expect(invoke).toHaveBeenCalledWith('update_source', { id: 2, muted: true })
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('bulk unmute calls API with muted=false', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    const sources = [createSource({ id: 1 })]
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources } })

    await wrapper.get('.btn-select').trigger('click')
    await wrapper.get('.btn-select-all').trigger('click')
    await (wrapper.vm as any).handleBulkMuteToggle(false)
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('update_source', { id: 1, muted: false })
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('bulk mute with empty selection does nothing', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    const wrapper = mount(MinimalSourceWithBulk, { props: { sources: [createSource({ id: 1 })] } })

    await wrapper.get('.btn-select').trigger('click')
    // Don't select anything — selection is empty
    await (wrapper.vm as any).handleBulkMuteToggle(true)
    await flushPromises()

    expect(invoke).not.toHaveBeenCalled()
    expect(wrapper.emitted('update')).toBeFalsy()
  })
})


describe('SourceTab header mode switch', () => {
  function mountSourceTab() {
    return mount(SourceTab, {
      props: {
        sources: [createComponentSource({ id: 1, owner: 'vuejs', repo: 'core' })],
        polling: false,
        unreadReleaseCounts: {},
        totalReleaseCounts: {},
        showSourceTypeIcons: true,
      },
      global: {
        stubs: { ContextMenu: true },
      },
    })
  }

  it('defaults to one sticky add-mode header row', () => {
    const wrapper = mountSourceTab()

    expect(wrapper.find('.source-sticky-panel').exists()).toBe(true)
    expect(wrapper.find('.source-header').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="source.placeholder"]').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="source.search"]').exists()).toBe(false)
    expect(wrapper.find('.btn-add-source').exists()).toBe(true)
    expect(wrapper.find('.btn-mode-toggle').exists()).toBe(true)
    expect(wrapper.find('.bulk-bar').exists()).toBe(false)
  })

  it('switches the same header row to search mode and back', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find('.source-header').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="source.search"]').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="source.placeholder"]').exists()).toBe(false)
    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find('input[placeholder="source.placeholder"]').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="source.search"]').exists()).toBe(false)
  })

  it('shows bulk actions in selection mode without requiring a second toolbar row', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.btn-select').trigger('click')
    expect(wrapper.find('.source-sticky-panel.has-bulk-bar').exists()).toBe(true)
    expect(wrapper.find('.bulk-bar').exists()).toBe(true)
    expect(wrapper.find('.source-checkbox').exists()).toBe(true)
    expect(wrapper.find('.source-toolbar').exists()).toBe(false)
    expect(wrapper.find('.source-toolbar-toggle').exists()).toBe(false)
  })

  it('renders sort field and direction groups in one dropdown', async () => {
    const wrapper = mountSourceTab()

    expect(wrapper.get('.sort-trigger').text()).toContain('source.sort_default')
    await wrapper.get('.sort-trigger').trigger('click')

    expect(wrapper.find('.sort-dropdown-divider').exists()).toBe(true)
    expect(wrapper.findAll('.sort-dropdown button').map(button => button.text())).toEqual([
      'source.sort_default',
      'source.sort_name',
      'source.sort_status',
      'source.sort_created',
      'source.sort_asc',
      'source.sort_desc',
    ])
  })

  it('sort trigger shows selected field text after selecting a field', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[3].trigger('click')

    expect(wrapper.get('.sort-trigger').text()).toContain('source.sort_created')
  })

  it('persists selected sort field and direction to localStorage', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[3].trigger('click')
    expect(window.localStorage.getItem('relwatch.source.sort.field')).toBe('created')

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[4].trigger('click')
    expect(window.localStorage.getItem('relwatch.source.sort.direction')).toBe('asc')
  })

  it('restores persisted sort field and direction on mount', () => {
    window.localStorage.setItem('relwatch.source.sort.field', 'created')
    window.localStorage.setItem('relwatch.source.sort.direction', 'asc')

    const wrapper = mountSourceTab()

    expect(wrapper.get('.sort-trigger').text()).toContain('source.sort_created')
    expect(wrapper.get('.sort-direction-icon').text()).toBe('↑')
  })
})
