import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import SourceTab from '../components/SourceTab.vue'
import { t } from '../i18n'
import { createSource } from './helpers'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ message: vi.fn(), confirm: vi.fn() }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText: vi.fn() }))

vi.mock('../api/sources', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/sources')>()
  return { ...actual, addSource: vi.fn(), removeSource: vi.fn(), updateSource: vi.fn() }
})
vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return { ...actual, checkSingleSource: vi.fn() }
})

afterEach(() => {
  vi.clearAllMocks()
  window.localStorage.clear()
})

/**
 * SourceTab 头部模式切换（添加 ↔ 搜索 + 排序 + 选择模式）真实组件测试。
 * 补充 source-tab.test.ts 未覆盖的头部行为。
 */
describe('SourceTab header mode switch', () => {
  function mountSourceTab() {
    return mount(SourceTab, {
      props: {
        sources: [createSource({ id: 1, owner: 'vuejs', repo: 'core' })],
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
    expect(wrapper.find(`input[placeholder="${t('source.placeholder')}"]`).exists()).toBe(true)
    expect(wrapper.find(`input[placeholder="${t('source.search')}"]`).exists()).toBe(false)
    expect(wrapper.find('.btn-add-source').exists()).toBe(true)
    expect(wrapper.find('.btn-mode-toggle').exists()).toBe(true)
    expect(wrapper.find('.bulk-bar').exists()).toBe(false)
  })

  it('switches the same header row to search mode and back', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find('.source-header').exists()).toBe(true)
    expect(wrapper.find(`input[placeholder="${t('source.search')}"]`).exists()).toBe(true)
    expect(wrapper.find(`input[placeholder="${t('source.placeholder')}"]`).exists()).toBe(false)
    await wrapper.get('.btn-mode-toggle').trigger('click')
    expect(wrapper.find(`input[placeholder="${t('source.placeholder')}"]`).exists()).toBe(true)
    expect(wrapper.find(`input[placeholder="${t('source.search')}"]`).exists()).toBe(false)
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

  it('renders sort field and direction groups in one dropdown（真实文案）', async () => {
    const wrapper = mountSourceTab()

    expect(wrapper.get('.sort-trigger').text()).toContain(t('source.sort_default'))
    await wrapper.get('.sort-trigger').trigger('click')

    expect(wrapper.find('.sort-dropdown-divider').exists()).toBe(true)
    expect(wrapper.findAll('.sort-dropdown button').map(button => button.text())).toEqual([
      t('source.sort_default'),
      t('source.sort_name'),
      t('source.sort_status'),
      t('source.sort_type'),
      t('source.sort_created'),
      t('source.sort_asc'),
      t('source.sort_desc'),
    ])
  })

  it('sort trigger shows selected field text after selecting a field', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[4].trigger('click')

    expect(wrapper.get('.sort-trigger').text()).toContain(t('source.sort_created'))
  })

  it('persists selected sort field and direction to localStorage', async () => {
    const wrapper = mountSourceTab()

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[4].trigger('click')
    expect(window.localStorage.getItem('relwatch.source.sort.field')).toBe('created')

    await wrapper.get('.sort-trigger').trigger('click')
    await wrapper.findAll('.sort-dropdown button')[5].trigger('click')
    expect(window.localStorage.getItem('relwatch.source.sort.direction')).toBe('asc')
  })

  it('restores persisted sort field and direction on mount', () => {
    window.localStorage.setItem('relwatch.source.sort.field', 'created')
    window.localStorage.setItem('relwatch.source.sort.direction', 'asc')

    const wrapper = mountSourceTab()

    expect(wrapper.get('.sort-trigger').text()).toContain(t('source.sort_created'))
    expect(wrapper.get('.sort-direction-icon').text()).toBe('↑')
  })
})
