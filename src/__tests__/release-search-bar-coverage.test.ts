import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ReleaseSearchBar from '../components/ReleaseSearchBar.vue'

vi.mock('../i18n', () => ({
  t: (key: string) => {
    const map: Record<string, string> = {
      'release.search': '搜索',
      'release.filter_all': '全部',
      'release.filter_unread': '未读',
      'release.filter_read': '已读',
      'release.importance_high': '高',
      'release.importance_medium': '中',
      'release.importance_low': '低',
      'release.view_simple': '简单',
      'release.view_aggregated': '聚合',
      'release.view_calendar': '日历',
      'tab.status': '状态',
      'tab.importance': '重要度',
      'input.clear': '清除',
    }
    return map[key] ?? key
  },
}))

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

beforeEach(() => {
  vi.clearAllMocks()
})

afterEach(() => {
  vi.useRealTimers()
})

/**
 * ReleaseSearchBar 补充测试
 *
 * 基于真实使用场景：
 * - 重要度筛选的完整交互（点击打开、选择选项、显示当前值）
 * - 悬停展开筛选（hover 进入区域打开，离开后延迟关闭）
 * - 状态下拉的所有选项（全部/已读）
 * - 键盘导航（Space/ArrowDown 打开，下拉内 ArrowDown/ArrowUp 移动焦点）
 * - 不同筛选值的正确显示
 */

describe('ReleaseSearchBar — 重要度筛选交互', () => {
  it('点击重要度筛选触发器打开下拉框', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    // 第二个 filter-field 是重要度
    await fields[1].find('.filter-trigger').trigger('click')

    expect(fields[1].find('.filter-dropdown').exists()).toBe(true)
    expect(fields[1].findAll('.filter-dropdown button')).toHaveLength(4)
  })

  it('选择"高"重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    // 第二个按钮是"高"（第一个是"全部"）
    await buttons[1].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['大'])
    expect(fields[1].find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择"中"重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    await buttons[2].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['中'])
  })

  it('选择"低"重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    await buttons[3].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['小'])
  })

  it('选择"全部"重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper({ importanceFilter: '大' })
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    await buttons[0].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['all'])
  })

  it('重要度筛选器显示当前选中的重要度值', () => {
    // 第一个 .filter-value 是状态，第二个是重要度
    const wrapperHigh = createWrapper({ importanceFilter: '大' })
    expect(wrapperHigh.findAll('.filter-value')[1].text()).toContain('高')

    const wrapperMed = createWrapper({ importanceFilter: '中' })
    expect(wrapperMed.findAll('.filter-value')[1].text()).toContain('中')

    const wrapperLow = createWrapper({ importanceFilter: '小' })
    expect(wrapperLow.findAll('.filter-value')[1].text()).toContain('低')

    const wrapperAll = createWrapper({ importanceFilter: 'all' })
    expect(wrapperAll.findAll('.filter-value')[1].text()).toContain('全部')
  })
})

describe('ReleaseSearchBar — 悬停展开筛选', () => {
  it('鼠标悬停在状态筛选区域展开下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].trigger('mouseenter')

    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)
  })

  it('鼠标悬停在重要度筛选区域展开下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].trigger('mouseenter')

    expect(fields[1].find('.filter-dropdown').exists()).toBe(true)
  })

  it('鼠标离开筛选组后延迟关闭下拉', async () => {
    vi.useFakeTimers()
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].trigger('mouseenter')
    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)

    // 离开整个筛选组
    wrapper.find('.filter-group').trigger('mouseleave')

    // 此时下拉还在（120ms 延迟）
    expect(wrapper.findAll('.filter-field')[0].find('.filter-dropdown').exists()).toBe(true)

    // 等待 120ms 后关闭
    vi.advanceTimersByTime(120)
    await wrapper.vm.$nextTick()

    expect(wrapper.findAll('.filter-field')[0].find('.filter-dropdown').exists()).toBe(false)
  })

  it('鼠标从筛选区域移入下拉菜单时保持展开', async () => {
    vi.useFakeTimers()
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].trigger('mouseenter')
    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)

    // 离开筛选区域（启动 120ms 定时器）
    await fields[0].trigger('mouseleave')

    // 在定时器触发前进入下拉菜单（清除定时器）
    fields[0].find('.filter-dropdown').trigger('mouseenter')

    // 即使等待超过 120ms，下拉仍然保持打开
    vi.advanceTimersByTime(150)
    await wrapper.vm.$nextTick()

    expect(wrapper.findAll('.filter-field')[0].find('.filter-dropdown').exists()).toBe(true)
  })

  it('从状态下拉切换到重要度下拉：悬停另一个区域自动切换', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    // 悬停状态区域
    await fields[0].trigger('mouseenter')
    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)
    expect(fields[1].find('.filter-dropdown').exists()).toBe(false)

    // 悬停重要度区域
    await fields[1].trigger('mouseenter')
    expect(fields[0].find('.filter-dropdown').exists()).toBe(false)
    expect(fields[1].find('.filter-dropdown').exists()).toBe(true)
  })
})

describe('ReleaseSearchBar — 状态筛选完整选项', () => {
  it('选择"全部"状态后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper({ statusFilter: 'unread' })
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('click')
    const buttons = fields[0].findAll('.filter-dropdown button')

    await buttons[0].trigger('click')

    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['all'])
    expect(fields[0].find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择"已读"状态后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('click')
    const buttons = fields[0].findAll('.filter-dropdown button')

    // 第三个按钮是"已读"
    await buttons[2].trigger('click')

    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['read'])
  })

  it('状态下拉显示不同状态的文本', () => {
    const wrapperAll = createWrapper({ statusFilter: 'all' })
    expect(wrapperAll.findAll('.filter-value')[0].text()).toContain('全部')

    const wrapperUnread = createWrapper({ statusFilter: 'unread' })
    expect(wrapperUnread.findAll('.filter-value')[0].text()).toContain('未读')

    const wrapperRead = createWrapper({ statusFilter: 'read' })
    expect(wrapperRead.findAll('.filter-value')[0].text()).toContain('已读')
  })
})

describe('ReleaseSearchBar — 下拉菜单内键盘导航', () => {
  it('下拉内按 ArrowDown 焦点移到下一个按钮', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    // 打开状态下拉
    await fields[0].find('.filter-trigger').trigger('click')
    const buttons = fields[0].findAll('.filter-dropdown button')

    // jsdom 中 focus() 不改变 activeElement，用原生事件 + 手动 focus 测试
    ;(buttons[0].element as HTMLElement).focus()
    buttons[0].element.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))

    // focus 被调度到 buttons[1]，jsdom 中通过 spy 验证
    const focusSpy = vi.spyOn(buttons[1].element as HTMLElement, 'focus')
    buttons[0].element.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))
    expect(focusSpy).toHaveBeenCalled()
  })

  it('下拉内按 ArrowUp 焦点移到上一个按钮', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('click')
    const buttons = fields[0].findAll('.filter-dropdown button')

    ;(buttons[1].element as HTMLElement).focus()
    const focusSpy = vi.spyOn(buttons[0].element as HTMLElement, 'focus')
    buttons[1].element.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }))
    expect(focusSpy).toHaveBeenCalled()
  })

  it('下拉内按 Escape 关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('click')
    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)

    const buttons = fields[0].findAll('.filter-dropdown button')
    ;(buttons[0].element as HTMLElement).focus()
    await buttons[0].trigger('keydown', { key: 'Escape' })

    expect(fields[0].find('.filter-dropdown').exists()).toBe(false)
  })
})

describe('ReleaseSearchBar — 筛选触发器键盘操作', () => {
  it('按 Space 键打开状态筛选', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('keydown', { key: ' ' })

    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)
  })

  it('按 ArrowDown 键打开状态筛选', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('keydown', { key: 'ArrowDown' })

    expect(fields[0].find('.filter-dropdown').exists()).toBe(true)
  })

  it('按 Space 键打开重要度筛选', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('keydown', { key: ' ' })

    expect(fields[1].find('.filter-dropdown').exists()).toBe(true)
  })
})

describe('ReleaseSearchBar — 组合筛选场景', () => {
  it('同时使用搜索、状态筛选和重要度筛选', async () => {
    const wrapper = createWrapper()

    // 输入搜索词
    await wrapper.find('input').setValue('vue')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['vue'])

    // 选择状态"未读"
    const fields = wrapper.findAll('.filter-field')
    await fields[0].find('.filter-trigger').trigger('click')
    await fields[0].findAll('.filter-dropdown button')[1].trigger('click')
    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['unread'])

    // 选择重要度"高"
    await fields[1].find('.filter-trigger').trigger('click')
    await fields[1].findAll('.filter-dropdown button')[1].trigger('click')
    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['大'])

    // 切换到日历视图
    const tabs = wrapper.findAll('.view-tabs button')
    await tabs[2].trigger('click')
    expect(wrapper.emitted('update:viewMode')?.[0]).toEqual(['calendar'])
  })
})
