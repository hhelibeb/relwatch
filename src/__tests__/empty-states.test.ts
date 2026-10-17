import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import EmptyState from '../components/common/EmptyState.vue'
import SkeletonList from '../components/common/SkeletonList.vue'
import ErrorBlock from '../components/common/ErrorBlock.vue'
import ReleaseSimpleList from '../components/ReleaseSimpleList.vue'
import { t } from '../i18n'

/**
 * 四态体系（空 / 载 / 错 / 成）的公共件与接线。
 *
 * 背景：11 处空态共用同一个 36px 半透明灰图标（与白底合成 ≈1.61:1），且空列表、
 * 加载中、拉取失败三种情况渲染同一个空态——「加载中」借用空态样式，「拉取失败」
 * 被说成「暂无数据」。这里钉住四件事：分三级文案与出口、骨架只在空列表时出现、
 * 失败走错误块而不是空态。
 */

describe('EmptyState', () => {
  it('渲染标题、说明与装饰图标，根节点保留 .empty 类', () => {
    const wrapper = mount(EmptyState, { props: { title: '标题', description: '说明' } })

    expect(wrapper.classes()).toContain('empty')
    expect(wrapper.find('.empty-title').text()).toBe('标题')
    expect(wrapper.find('.empty-desc').text()).toBe('说明')
    // 装饰图标不进无障碍树，空态语义由标题承担
    expect(wrapper.find('.empty-icon').attributes('aria-hidden')).toBe('true')
  })

  it('无说明时不渲染说明行', () => {
    const wrapper = mount(EmptyState, { props: { title: '标题' } })
    expect(wrapper.find('.empty-desc').exists()).toBe(false)
  })

  it('actions 插槽有内容才渲染动作区', () => {
    const withoutActions = mount(EmptyState, { props: { title: '标题' } })
    expect(withoutActions.find('.empty-actions').exists()).toBe(false)

    const withActions = mount(EmptyState, {
      props: { title: '标题' },
      slots: { actions: '<button class="cta">出口</button>' },
    })
    expect(withActions.find('.empty-actions .cta').exists()).toBe(true)
  })

  it('紧凑档收起图标，供弹窗内的小容器使用', () => {
    const wrapper = mount(EmptyState, { props: { title: '标题', compact: true } })
    expect(wrapper.classes()).toContain('empty-compact')
    expect(wrapper.find('.empty-icon').exists()).toBe(false)
  })
})

describe('SkeletonList', () => {
  it('按 rows 渲染骨架条，并以 aria-busy + label 暴露加载语义', () => {
    const wrapper = mount(SkeletonList, { props: { rows: 3, label: '加载中' } })

    expect(wrapper.findAll('.skeleton-item')).toHaveLength(3)
    expect(wrapper.attributes('role')).toBe('status')
    expect(wrapper.attributes('aria-busy')).toBe('true')
    expect(wrapper.attributes('aria-label')).toBe('加载中')
  })

  it('card 变体没有头像位（版本卡是纯文本块）', () => {
    const row = mount(SkeletonList, { props: { rows: 1, label: '加载中' } })
    expect(row.find('.skeleton-avatar').exists()).toBe(true)

    const card = mount(SkeletonList, { props: { rows: 2, variant: 'card', label: '加载中' } })
    expect(card.find('.skeleton-avatar').exists()).toBe(false)
    expect(card.findAll('.skeleton-item')).toHaveLength(2)
  })
})

describe('ErrorBlock', () => {
  it('原位显示原因，重试按钮发出 retry', async () => {
    const wrapper = mount(ErrorBlock, { props: { message: '拉取失败', retryLabel: '重试' } })

    expect(wrapper.attributes('role')).toBe('alert')
    expect(wrapper.find('.error-block-msg').text()).toBe('拉取失败')

    await wrapper.find('.error-block-retry').trigger('click')
    expect(wrapper.emitted('retry')).toHaveLength(1)
  })
})

describe('ReleaseSimpleList — 空态分三级', () => {
  function mountList(props: Record<string, unknown> = {}) {
    return mount(ReleaseSimpleList, {
      props: { releases: [], isFiltering: false, hasSearchQuery: false, deepSearch: false, ...props },
    })
  }

  it('首次为空：给价值说明，不放清空类动作', () => {
    const wrapper = mountList()

    expect(wrapper.find('.empty').text()).toContain(t('release.empty'))
    expect(wrapper.find('.empty-desc').text()).toContain(t('release.empty_desc'))
    expect(wrapper.find('.empty-actions').exists()).toBe(false)
  })

  it('筛选无结果：说明 + 清空筛选出口', async () => {
    const wrapper = mountList({ isFiltering: true })

    expect(wrapper.find('.empty').text()).toContain(t('release.no_match'))
    const actions = wrapper.findAll('.empty-actions button')
    expect(actions.map(b => b.text())).toEqual([t('release.filter_clear_all')])

    await actions[0].trigger('click')
    expect(wrapper.emitted('clear-filters')).toHaveLength(1)
  })

  it('搜索无结果：深度搜索与清空搜索两个出口', async () => {
    const wrapper = mountList({ hasSearchQuery: true, isFiltering: true })

    expect(wrapper.find('.empty-desc').text()).toContain(t('release.deep_search_label'))
    const actions = wrapper.findAll('.empty-actions button')
    expect(actions.map(b => b.text())).toEqual([
      t('release.deep_search_label'),
      t('common.clear_search'),
    ])

    await actions[0].trigger('click')
    await actions[1].trigger('click')
    expect(wrapper.emitted('enable-deep')).toHaveLength(1)
    expect(wrapper.emitted('clear-search')).toHaveLength(1)
  })

  it('加载中且列表为空：渲染骨架而不是空态', () => {
    const wrapper = mountList({ loading: true })

    expect(wrapper.find('.skeleton').exists()).toBe(true)
    expect(wrapper.find('.empty').exists()).toBe(false)
  })
})
