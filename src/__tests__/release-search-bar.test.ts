import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, shallowMount, type VueWrapper } from '@vue/test-utils'
import ReleaseTab from '../components/ReleaseTab.vue'
import ReleaseSearchBar from '../components/ReleaseSearchBar.vue'
import ReleaseItem from '../components/ReleaseItem.vue'
import { t } from '../i18n'
import { createRelease } from './helpers'
import type { ReleaseInfo } from '../api/releases'

/**
 * ReleaseSearchBar.vue 行为测试（真实 i18n 字典，不 mock）
 *
 * 组件为受控模式：状态由 props 传入、通过 emit 传出。
 * 按行为分组：搜索框 / 视图切换 / 筛选下拉 / 来源筛选 / 悬停 / 键盘导航 / 组合场景。
 * 末尾附 ReleaseTab 集成（ReleaseSearchBar 实例化契约）。
 */
function createWrapper(props: Record<string, unknown> = {}) {
  return mount(ReleaseSearchBar, {
    props: {
      modelValue: '',
      statusFilter: 'all',
      importanceFilter: 'all',
      sourceFilter: 'all',
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

// ============ 搜索框 ============

describe('ReleaseSearchBar — 搜索框', () => {
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

// ============ 版本计数徽标 ============

describe('ReleaseSearchBar — 版本计数徽标', () => {
  it('传入 count 时在搜索框内渲染数字徽标', () => {
    const wrapper = createWrapper({ count: 128 })

    expect(wrapper.find('.release-count').exists()).toBe(true)
    expect(wrapper.find('.release-count').text()).toBe('(128)')
  })

  it('count 徽标显示「N 个版本」的悬停提示', () => {
    const wrapper = createWrapper({ count: 128 })

    expect(wrapper.find('.release-count').attributes('title')).toBe(t('release.versions', '128'))
  })

  it('未传 count 时不渲染徽标', () => {
    const wrapper = createWrapper()

    expect(wrapper.find('.release-count').exists()).toBe(false)
  })

  it('无输入时徽标贴右缘；有输入时让位给清空按钮', () => {
    const wrapperEmpty = createWrapper({ count: 128 })
    expect(wrapperEmpty.find('.release-count').classes()).not.toContain('has-clear')

    const wrapperWithInput = createWrapper({ count: 128, modelValue: 'vue' })
    expect(wrapperWithInput.find('.release-count').classes()).toContain('has-clear')
  })
})

// ============ 视图切换（折叠下拉） ============

describe('ReleaseSearchBar — 视图切换（折叠下拉）', () => {
  // 视图下拉是第 4 个筛选字段（状态/重要度/来源/视图）
  function openViewDropdown(wrapper: ReturnType<typeof createWrapper>) {
    return wrapper.findAll('.filter-field')[3]
  }

  it('默认 simple 视图选项选中', async () => {
    const wrapper = createWrapper()

    const viewField = openViewDropdown(wrapper)
    await viewField.find('.filter-trigger').trigger('click')

    const items = viewField.findAll('.filter-dropdown button')
    expect(items[0].classes()).toContain('selected')
    expect(items[1].classes()).not.toContain('selected')
    expect(items[2].classes()).not.toContain('selected')
  })

  it('点击聚合视图选项 emit update:viewMode("aggregated")', async () => {
    const wrapper = createWrapper()

    const viewField = openViewDropdown(wrapper)
    await viewField.find('.filter-trigger').trigger('click')
    await viewField.findAll('.filter-dropdown button')[1].trigger('click')

    expect(wrapper.emitted('update:viewMode')?.[0]).toEqual(['aggregated'])
  })

  it('点击日历视图选项 emit update:viewMode("calendar")', async () => {
    const wrapper = createWrapper()

    const viewField = openViewDropdown(wrapper)
    await viewField.find('.filter-trigger').trigger('click')
    await viewField.findAll('.filter-dropdown button')[2].trigger('click')

    expect(wrapper.emitted('update:viewMode')?.[0]).toEqual(['calendar'])
  })

  it('当前视图选项高亮', async () => {
    const wrapper = createWrapper({ viewMode: 'aggregated' })

    const viewField = openViewDropdown(wrapper)
    await viewField.find('.filter-trigger').trigger('click')

    const items = viewField.findAll('.filter-dropdown button')
    expect(items[1].classes()).toContain('selected')
    expect(items[0].classes()).not.toContain('selected')
    expect(items[2].classes()).not.toContain('selected')
  })

  it('触发按钮与下拉选项均显示视图图标', async () => {
    const wrapper = createWrapper({ viewMode: 'calendar' })

    const viewField = openViewDropdown(wrapper)
    // 触发按钮图标：与旧按钮组同源（list/grid/calendar）
    expect(viewField.find('.filter-trigger svg use').attributes('href')).toBe('/icons.svg#calendar-icon')

    await viewField.find('.filter-trigger').trigger('click')
    const optionIcons = viewField.findAll('.filter-dropdown button .filter-type-icon svg use').map(u => u.attributes('href'))
    expect(optionIcons).toEqual(['/icons.svg#list-icon', '/icons.svg#grid-icon', '/icons.svg#calendar-icon'])
  })
})

// ============ 状态筛选 ============

describe('ReleaseSearchBar — 状态筛选', () => {
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

  it('选择「全部」状态后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper({ statusFilter: 'unread' })
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('click')
    const buttons = fields[0].findAll('.filter-dropdown button')

    await buttons[0].trigger('click')

    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['all'])
    expect(fields[0].find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择「已读」状态后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[0].find('.filter-trigger').trigger('click')
    const buttons = fields[0].findAll('.filter-dropdown button')

    await buttons[2].trigger('click')

    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['read'])
  })

  it('状态下拉显示不同状态的文本（真实字典）', () => {
    const wrapperAll = createWrapper({ statusFilter: 'all' })
    expect(wrapperAll.findAll('.filter-value')[0].text()).toContain(t('release.filter_all'))

    const wrapperUnread = createWrapper({ statusFilter: 'unread' })
    expect(wrapperUnread.findAll('.filter-value')[0].text()).toContain(t('release.filter_unread'))

    const wrapperRead = createWrapper({ statusFilter: 'read' })
    expect(wrapperRead.findAll('.filter-value')[0].text()).toContain(t('release.filter_read'))
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

// ============ 重要度筛选 ============

describe('ReleaseSearchBar — 重要度筛选交互', () => {
  it('点击重要度筛选触发器打开下拉框', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    // 第二个 filter-field 是重要度
    await fields[1].find('.filter-trigger').trigger('click')

    expect(fields[1].find('.filter-dropdown').exists()).toBe(true)
    expect(fields[1].findAll('.filter-dropdown button')).toHaveLength(4)
  })

  it('选择「大」重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    // 第二个按钮是"大"（第一个是"全部"）
    await buttons[1].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['大'])
    expect(fields[1].find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择「中」重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    await buttons[2].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['中'])
  })

  it('选择「小」重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper()
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    await buttons[3].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['小'])
  })

  it('选择「全部」重要度后 emit 并关闭下拉', async () => {
    const wrapper = createWrapper({ importanceFilter: '大' })
    const fields = wrapper.findAll('.filter-field')

    await fields[1].find('.filter-trigger').trigger('click')
    const buttons = fields[1].findAll('.filter-dropdown button')

    await buttons[0].trigger('click')

    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['all'])
  })

  it('重要度筛选器显示当前选中的重要度值（DOM 入口，真实字典）', () => {
    // 第一个 .filter-value 是状态，第二个是重要度
    const wrapperHigh = createWrapper({ importanceFilter: '大' })
    expect(wrapperHigh.findAll('.filter-value')[1].text()).toContain(t('release.importance_high'))

    const wrapperMed = createWrapper({ importanceFilter: '中' })
    expect(wrapperMed.findAll('.filter-value')[1].text()).toContain(t('release.importance_medium'))

    const wrapperLow = createWrapper({ importanceFilter: '小' })
    expect(wrapperLow.findAll('.filter-value')[1].text()).toContain(t('release.importance_low'))

    const wrapperAll = createWrapper({ importanceFilter: 'all' })
    expect(wrapperAll.findAll('.filter-value')[1].text()).toContain(t('release.filter_all'))
  })
})

// ============ 来源筛选 ============

describe('ReleaseSearchBar — 来源筛选', () => {
  // 来源下拉是第 3 个筛选字段（状态/重要度/来源/视图）
  function sourceField(wrapper: ReturnType<typeof createWrapper>) {
    return wrapper.findAll('.filter-field')[2]
  }

  it('来源下拉选项包含全部监控类型（真实字典文案）', async () => {
    const wrapper = createWrapper()

    const field = sourceField(wrapper)
    await field.find('.filter-trigger').trigger('click')

    const items = field.findAll('.filter-dropdown button')
    expect(items.map(i => i.text())).toEqual([
      t('release.filter_all'),
      t('source.type_github'),
      t('source.type_huggingface'),
      t('source.type_youtube'),
      t('source.type_bilibili'),
    ])
  })

  it('选择 YouTube 选项 emit update:sourceFilter("youtube") 并关闭', async () => {
    const wrapper = createWrapper()

    const field = sourceField(wrapper)
    await field.find('.filter-trigger').trigger('click')
    await field.findAll('.filter-dropdown button')[3].trigger('click')

    expect(wrapper.emitted('update:sourceFilter')?.[0]).toEqual(['youtube'])
    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)
  })

  it('选中来源时触发按钮显示类型标题与图标', async () => {
    const wrapper = createWrapper({ sourceFilter: 'bilibili' })

    const trigger = sourceField(wrapper).find('.filter-trigger')
    expect(trigger.text()).toContain(t('source.type_bilibili'))
    expect(trigger.find('svg use').attributes('href')).toBe('/icons.svg#bilibili-icon')
  })
})

// ============ 悬停展开 ============

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

// ============ 下拉菜单内键盘导航 ============

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

// ============ 筛选触发器键盘操作 ============

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

// ============ 组合筛选场景 ============

describe('ReleaseSearchBar — 组合筛选场景', () => {
  it('同时使用搜索、状态筛选、重要度筛选并切换视图', async () => {
    const wrapper = createWrapper()

    // 输入搜索词
    await wrapper.find('input').setValue('vue')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['vue'])

    // 选择状态"未读"
    const fields = wrapper.findAll('.filter-field')
    await fields[0].find('.filter-trigger').trigger('click')
    await fields[0].findAll('.filter-dropdown button')[1].trigger('click')
    expect(wrapper.emitted('update:statusFilter')?.[0]).toEqual(['unread'])

    // 选择重要度"大"
    await fields[1].find('.filter-trigger').trigger('click')
    await fields[1].findAll('.filter-dropdown button')[1].trigger('click')
    expect(wrapper.emitted('update:importanceFilter')?.[0]).toEqual(['大'])

    // 切换到日历视图（视图已折叠为下拉，位于第 4 个筛选字段）
    const viewField = wrapper.findAll('.filter-field')[3]
    await viewField.find('.filter-trigger').trigger('click')
    await viewField.findAll('.filter-dropdown button')[2].trigger('click')
    expect(wrapper.emitted('update:viewMode')?.[0]).toEqual(['calendar'])
  })
})

// ============ ReleaseTab 集成（ReleaseSearchBar 实例化契约） ============

type ReleaseTabTestProps = {
  releases: ReleaseInfo[]
  search?: string
  statusFilter?: 'all' | 'unread' | 'read'
}

function releaseProp(item: VueWrapper): ReleaseInfo {
  return (item.props() as { release: ReleaseInfo }).release
}

async function setReleaseTabProps(wrapper: ReturnType<typeof createReleaseTabWrapper>, props: Partial<ReleaseTabTestProps>) {
  await wrapper.setProps(props as Parameters<typeof wrapper.setProps>[0])
}

function createReleaseTabWrapper(releases: ReleaseInfo[] = [], props: Partial<ReleaseTabTestProps> = {}) {
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
    const wrapper = createReleaseTabWrapper()
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })

  it('聚合视图：恰好渲染 1 个 ReleaseSearchBar', async () => {
    const wrapper = createReleaseTabWrapper()
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'aggregated')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })

  it('日历主视图：恰好渲染 1 个 ReleaseSearchBar', async () => {
    const wrapper = createReleaseTabWrapper()
    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:viewMode', 'calendar')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAllComponents(ReleaseSearchBar)).toHaveLength(1)
  })

  it('日历钻取视图：恰好渲染 1 个 ReleaseSearchBar（不重复）', async () => {
    const today = new Date()
    const wrapper = createReleaseTabWrapper([createRelease({ published_at: today.toISOString() })])
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
    const wrapper = createReleaseTabWrapper([createRelease({ published_at: today.toISOString() })])
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
    const wrapper = createReleaseTabWrapper(createProtectiveReleases(), { search: 'tauri' })
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(2)

    wrapper.findComponent(ReleaseSearchBar).vm.$emit('update:modelValue', 'vue')
    expect(wrapper.emitted('update:search')?.[0]).toEqual(['vue'])

    await setReleaseTabProps(wrapper, { search: 'vue' })
    expect(wrapper.findAllComponents(ReleaseItem)).toHaveLength(1)
    expect(releaseProp(wrapper.findComponent(ReleaseItem))).toMatchObject({ id: 2 })
  })

  it('状态过滤覆盖全部 / 未读 / 已读', async () => {
    const wrapper = createReleaseTabWrapper(createProtectiveReleases())
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
    const wrapper = createReleaseTabWrapper(createProtectiveReleases())
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
    const wrapper = createReleaseTabWrapper(createProtectiveReleases())
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
    const wrapper = createReleaseTabWrapper(createProtectiveReleases())
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
    const wrapper = createReleaseTabWrapper([
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
    const wrapper = createReleaseTabWrapper(createProtectiveReleases())
    wrapper.findComponent(ReleaseItem).vm.$emit('update')
    await wrapper.vm.$nextTick()
    expect(wrapper.emitted('update')).toEqual([[]])
  })
})
