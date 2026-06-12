import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ReleaseSearchBar from '../components/ReleaseSearchBar.vue'

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => {
    const map: Record<string, string> = {
      'release.search': '搜索版本...',
      'release.filter_all': '全部',
      'release.filter_unread': '未读',
      'release.filter_read': '已读',
      'release.importance_high': '高',
      'release.importance_medium': '中',
      'release.importance_low': '低',
      'tab.status': '状态',
      'tab.importance': '重要度',
      'input.clear': '清除',
    }
    return map[key] ?? key
  }),
}))

beforeEach(() => {
  vi.clearAllMocks()
})

/**
 * ReleaseSearchBar.vue 真实运行场景测试
 *
 * 该组件提供版本列表页的搜索框 + 状态/重要度筛选 + 视图切换。
 * 所有状态通过 props 传入、通过 emit 传出（受控组件模式）。
 */
describe('ReleaseSearchBar.vue — 搜索框', () => {
  function createWrapper(props: Record<string, unknown> = {}) {
    return mount(ReleaseSearchBar, {
      props: {
        modelValue: '',
        statusFilter: 'all',
        importanceFilter: 'all',
        viewMode: 'simple',
        ...props,
      },
    })
  }

  it('输入时 emit update:modelValue', async () => {
    const wrapper = createWrapper()

    const input = wrapper.find('input')
    await input.setValue('tauri')

    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['tauri'])
  })

  it('Enter 键触发 searchEnter', async () => {
    const wrapper = createWrapper({ modelValue: 'test' })

    await wrapper.find('input').trigger('keydown.enter.prevent')

    expect(wrapper.emitted('searchEnter')).toBeTruthy()
  })

  it('modelValue 非空时显示清空按钮', () => {
    const wrapper = createWrapper({ modelValue: 'something' })

    expect(wrapper.find('.input-clear-btn').exists()).toBe(true)
  })

  it('modelValue 为空时隐藏清空按钮', () => {
    const wrapper = createWrapper({ modelValue: '' })

    expect(wrapper.find('.input-clear-btn').exists()).toBe(false)
  })

  it('点击清空按钮 emit update:modelValue("")', async () => {
    const wrapper = createWrapper({ modelValue: 'tauri' })

    await wrapper.find('.input-clear-btn').trigger('click')

    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([''])
  })

  it('showSearch=false 时隐藏搜索框', () => {
    const wrapper = createWrapper({ showSearch: false })

    expect(wrapper.find('.input-clear-wrap').exists()).toBe(false)
  })

  it('showSearch 默认为 true（不传时显示）', () => {
    const wrapper = createWrapper()

    expect(wrapper.find('.input-clear-wrap').exists()).toBe(true)
  })
})

describe('ReleaseSearchBar.vue — 视图切换', () => {
  function createWrapper(props: Record<string, unknown> = {}) {
    return mount(ReleaseSearchBar, {
      props: {
        modelValue: '',
        statusFilter: 'all',
        importanceFilter: 'all',
        viewMode: 'simple',
        ...props,
      },
    })
  }

  it('默认 simple 视图高亮', () => {
    const wrapper = createWrapper()

    const tabs = wrapper.findAll('.view-tabs button')
    expect(tabs[0].classes()).toContain('active')
    expect(tabs[1].classes()).not.toContain('active')
    expect(tabs[2].classes()).not.toContain('active')
  })

  it('点击聚合视图按钮 emit update:viewMode("aggregated")', async () => {
    const wrapper = createWrapper()

    await wrapper.findAll('.view-tabs button')[1].trigger('click')

    expect(wrapper.emitted('update:viewMode')?.[0]).toEqual(['aggregated'])
  })

  it('点击日历视图按钮 emit update:viewMode("calendar")', async () => {
    const wrapper = createWrapper()

    await wrapper.findAll('.view-tabs button')[2].trigger('click')

    expect(wrapper.emitted('update:viewMode')?.[0]).toEqual(['calendar'])
  })

  it('当前视图按钮高亮', () => {
    const wrapper = createWrapper({ viewMode: 'aggregated' })

    const tabs = wrapper.findAll('.view-tabs button')
    expect(tabs[1].classes()).toContain('active')
    expect(tabs[0].classes()).not.toContain('active')
    expect(tabs[2].classes()).not.toContain('active')
  })
})

describe('ReleaseSearchBar.vue — 筛选下拉', () => {
  function createWrapper(props: Record<string, unknown> = {}) {
    return mount(ReleaseSearchBar, {
      props: {
        modelValue: '',
        statusFilter: 'all',
        importanceFilter: 'all',
        viewMode: 'simple',
        ...props,
      },
    })
  }

  it('点击状态筛选触发按钮切换 dropdown 显示', async () => {
    const wrapper = createWrapper()

    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(true)

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择状态选项后 emit update:statusFilter 并关闭', async () => {
    const wrapper = createWrapper()

    await wrapper.find('.filter-trigger').trigger('click')
    const items = wrapper.findAll('.filter-dropdown button')
    await items[1].trigger('click') // "未读"

    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['unread'])
    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)
  })

  it('importanceDisplayText 计算属性', () => {
    const wrapperAll = createWrapper({ importanceFilter: 'all' })
    expect((wrapperAll.vm as any).importanceDisplayText).toContain('全部')

    const wrapperHigh = createWrapper({ importanceFilter: '大' })
    expect((wrapperHigh.vm as any).importanceDisplayText).toContain('高')

    const wrapperMed = createWrapper({ importanceFilter: '中' })
    expect((wrapperMed.vm as any).importanceDisplayText).toContain('中')

    const wrapperLow = createWrapper({ importanceFilter: '小' })
    expect((wrapperLow.vm as any).importanceDisplayText).toContain('低')
  })

  it('键盘 Enter 展开筛选', async () => {
    const wrapper = createWrapper()

    await wrapper.find('.filter-trigger').trigger('keydown', { key: 'Enter' })

    expect(wrapper.find('.filter-dropdown').exists()).toBe(true)
  })

  it('键盘 Escape 关闭筛选', async () => {
    const wrapper = createWrapper()

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(true)

    await wrapper.find('.filter-trigger').trigger('keydown', { key: 'Escape' })

    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)
  })
})
