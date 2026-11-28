import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { enableAutoUnmount, mount, flushPromises } from '@vue/test-utils'
import { nextTick } from 'vue'
import ReleaseTab from '../components/ReleaseTab.vue'
import { useUiStore } from '../stores/ui'
import { t } from '../i18n'
import { createRelease } from './helpers'
import { confirm } from '@tauri-apps/plugin-dialog'
import {
  batchUpdateReleaseStatus,
  batchSetReleaseFlag,
  batchDeleteReleases,
} from '../api/releases'

vi.mock('@tauri-apps/plugin-dialog', () => ({ confirm: vi.fn() }))
vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return {
    ...actual,
    batchUpdateReleaseStatus: vi.fn(),
    batchSetReleaseFlag: vi.fn(),
    batchDeleteReleases: vi.fn(),
    getReleaseDetail: vi.fn(),
  }
})

const confirmMock = vi.mocked(confirm)
const batchStatusMock = vi.mocked(batchUpdateReleaseStatus)
const batchFlagMock = vi.mocked(batchSetReleaseFlag)
const batchDeleteMock = vi.mocked(batchDeleteReleases)

function mountReleaseTab(releases = [
  createRelease({ id: 1, owner: 'vuejs', repo: 'core', tag_name: 'v3.5.0', published_at: '2025-03-01T00:00:00Z', notification_status: 'pending' }),
  createRelease({ id: 2, owner: 'vuejs', repo: 'core', tag_name: 'v3.4.0', published_at: '2025-02-01T00:00:00Z', notification_status: 'pending' }),
  createRelease({ id: 3, owner: 'microsoft', repo: 'vscode', tag_name: 'v1.90.0', published_at: '2025-01-01T00:00:00Z', notification_status: 'clicked' }),
], extraProps: Record<string, unknown> = {}) {
  const ui = useUiStore()
  const showToast = vi.spyOn(ui, 'showToast')
  const wrapper = mount(ReleaseTab, {
    props: { releases, active: true, ...extraProps },
    global: {
      stubs: { ContextMenu: true, MarkdownContent: true },
    },
    attachTo: document.body,
  })
  return { wrapper, showToast }
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
  confirmMock.mockResolvedValue(true as never)
  batchStatusMock.mockResolvedValue(2)
  batchFlagMock.mockResolvedValue(2)
  batchDeleteMock.mockResolvedValue(2)
})

enableAutoUnmount(afterEach)

describe('ReleaseTab — 批量操作与全部标为已读', () => {
  it('默认不显示多选栏与勾选框，工具栏展示紧凑图标按钮', () => {
    const { wrapper } = mountReleaseTab()
    expect(wrapper.find('.release-bulk-bar').exists()).toBe(false)
    expect(wrapper.find('.release-select-box').exists()).toBe(false)
    expect(wrapper.find('.mark-all-read-btn').exists()).toBe(true)
    expect(wrapper.find('.select-mode-btn').exists()).toBe(true)
  })

  it('全部标为已读：当前筛选无未读时禁用，有未读时弹窗确认并仅标记当前筛选范围内未读', async () => {
    const { wrapper: emptyUnread } = mountReleaseTab([
      createRelease({ id: 1, notification_status: 'clicked' }),
    ])
    expect(emptyUnread.get('.mark-all-read-btn').attributes('disabled')).toBeDefined()

    const { wrapper, showToast } = mountReleaseTab(undefined, { search: 'vuejs' })
    await wrapper.get('.mark-all-read-btn').trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledWith(
      t('release.mark_all_read_filtered_confirm', '2'),
      expect.any(Object),
    )
    expect(batchStatusMock).toHaveBeenCalledWith([1, 2], 'clicked')
    expect(showToast).toHaveBeenCalledWith(t('release.bulk_marked_read', '2'))
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('进入选择模式后可全选并批量标为已读、忽略、打旗标、删除', async () => {
    const { wrapper, showToast } = mountReleaseTab()

    await wrapper.get('.select-mode-btn').trigger('click')
    expect(wrapper.find('.release-bulk-bar').exists()).toBe(true)
    expect(wrapper.findAll('.release-select-box')).toHaveLength(3)

    // Select all
    await wrapper.findAll('.bulk-link-btn')[0].trigger('click')
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '3'))

    // Batch mark read
    const actionBtns = wrapper.findAll('.bulk-action-btn')
    await actionBtns[0].trigger('click')
    await flushPromises()
    expect(batchStatusMock).toHaveBeenCalledWith([1, 2, 3], 'clicked')

    // Batch ignore
    await actionBtns[1].trigger('click')
    await flushPromises()
    expect(batchStatusMock).toHaveBeenCalledWith([1, 2, 3], 'ignored')

    // Batch flag via swatch popover
    await actionBtns[2].trigger('click')
    expect(wrapper.find('.bulk-flag-popover').exists()).toBe(true)
    await wrapper.findAll('.bulk-flag-swatch')[0].trigger('click')
    await flushPromises()
    expect(batchFlagMock).toHaveBeenCalledWith([1, 2, 3], 1)
    expect(wrapper.find('.bulk-flag-popover').exists()).toBe(false)

    // Batch delete with confirm
    await wrapper.get('.bulk-action-danger').trigger('click')
    await flushPromises()
    expect(confirmMock).toHaveBeenCalledWith(
      t('release.bulk_delete_confirm', '3'),
      expect.any(Object),
    )
    expect(batchDeleteMock).toHaveBeenCalledWith([1, 2, 3])
    expect(showToast).toHaveBeenCalledWith(t('release.bulk_deleted', '2'))
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '0'))
  })

  it('按 Esc 先关旗标弹层再退出选择模式', async () => {
    const { wrapper } = mountReleaseTab()
    await wrapper.get('.select-mode-btn').trigger('click')
    await wrapper.findAll('.bulk-link-btn')[0].trigger('click')
    await wrapper.findAll('.bulk-action-btn')[2].trigger('click')
    expect(wrapper.find('.bulk-flag-popover').exists()).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(wrapper.find('.bulk-flag-popover').exists()).toBe(false)
    expect(wrapper.find('.release-bulk-bar').exists()).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(wrapper.find('.release-bulk-bar').exists()).toBe(false)
  })

  it('聚合视图组头支持按源全部标为已读', async () => {
    localStorage.setItem('relwatch.release.view', JSON.stringify({ viewMode: 'aggregated' }))
    const { wrapper } = mountReleaseTab()
    await nextTick()
    expect(wrapper.findAll('.repo-group').length).toBe(2)

    const repoReadBtns = wrapper.findAll('.repo-mark-read-btn')
    // Only vuejs/core has unread releases; microsoft/vscode has 0 unread
    expect(repoReadBtns).toHaveLength(1)

    await repoReadBtns[0].trigger('click')
    await flushPromises()

    expect(confirmMock).toHaveBeenCalledWith(
      t('release.mark_source_read_confirm', 'vuejs/core', '2'),
      expect.any(Object),
    )
    expect(batchStatusMock).toHaveBeenCalledWith([1, 2], 'clicked')
  })

  it('聚合视图：全选与 Shift 连选只看已展开分组内的版本', async () => {
    localStorage.setItem('relwatch.release.view', JSON.stringify({ viewMode: 'aggregated' }))
    const { wrapper } = mountReleaseTab()
    await nextTick()

    // 组按最新版本降序：只展开 vuejs/core（版本 1、2），microsoft/vscode 保持折叠
    await wrapper.findAll('.repo-group-toggle')[0].trigger('click')
    await wrapper.get('.select-mode-btn').trigger('click')
    expect(wrapper.findAll('.release-select-box')).toHaveLength(2)

    await wrapper.findAll('.bulk-link-btn')[0].trigger('click')
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '2'))

    // 折叠该分组后，已选项随之剪枝（折叠的版本不属可见范围）
    await wrapper.findAll('.repo-group-toggle')[0].trigger('click')
    await nextTick()
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '0'))
  })

  it('Shift + 点击支持区间连续勾选，且单次点击后不残留 shift 状态', async () => {
    const { wrapper } = mountReleaseTab([
      createRelease({ id: 1, published_at: '2025-04-01T00:00:00Z' }),
      createRelease({ id: 2, published_at: '2025-03-01T00:00:00Z' }),
      createRelease({ id: 3, published_at: '2025-02-01T00:00:00Z' }),
      createRelease({ id: 4, published_at: '2025-01-01T00:00:00Z' }),
    ])

    await wrapper.get('.select-mode-btn').trigger('click')
    const boxes = wrapper.findAll('.release-select-box input[type="checkbox"]')

    await boxes[0].trigger('click', { shiftKey: false })
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '1'))

    await boxes[2].trigger('click', { shiftKey: true })
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '3'))

    await boxes[3].trigger('click', { shiftKey: false })
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '4'))
  })

  it('搜索或筛选变化时自动剪枝不再可见的已选项', async () => {
    const { wrapper } = mountReleaseTab()
    await wrapper.get('.select-mode-btn').trigger('click')
    await wrapper.findAll('.bulk-link-btn')[0].trigger('click')
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '3'))

    await wrapper.setProps({ search: 'vscode' } as Parameters<typeof wrapper.setProps>[0])
    await nextTick()
    expect(wrapper.get('.bulk-count').text()).toBe(t('release.bulk_count', '1'))
  })

  it('切换到日历月网格视图时自动退出选择模式并隐藏选择开关', async () => {
    const { wrapper } = mountReleaseTab()
    await wrapper.get('.select-mode-btn').trigger('click')
    expect(wrapper.find('.release-bulk-bar').exists()).toBe(true)

    const viewField = wrapper.findAll('.filter-field')[3]
    await viewField.find('.filter-trigger').trigger('click')
    await viewField.findAll('.filter-dropdown button')[2].trigger('click')
    await nextTick()

    expect(wrapper.find('.release-bulk-bar').exists()).toBe(false)
    expect(wrapper.find('.select-mode-btn').exists()).toBe(false)
  })
})

