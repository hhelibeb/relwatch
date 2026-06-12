import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import ContextMenu from '../components/common/ContextMenu.vue'

// 注意：ContextMenu.vue 中的 import 是 '../../i18n'
// vi.mock 路径基于测试文件位置解析：从 src/__tests__/ 出发
vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => {
    const map: Record<string, string> = {
      'context.open': '打开',
      'context.copy_link': '复制链接',
    }
    return map[key] ?? key
  }),
}))

beforeEach(() => {
  vi.clearAllMocks()
})

/**
 * ContextMenu.vue 真实运行场景测试
 *
 * 固定定位右键菜单，支持：
 * - 无 items prop → 默认渲染 "打开" 和 "复制链接"
 * - 有 items prop → 按 items 渲染按钮
 * - 挂载后自动聚焦第一个按钮
 * - 键盘导航：ArrowDown/ArrowUp（循环）、Escape → close
 */
describe('ContextMenu.vue — 渲染', () => {
  function mountMenu(props: Record<string, unknown> = {}) {
    return mount(ContextMenu, {
      props: { x: 100, y: 200, ...props },
    })
  }

  it('无 items prop 时默认渲染两个按钮', () => {
    const wrapper = mountMenu()

    const buttons = wrapper.findAll('button')
    expect(buttons).toHaveLength(2)
    expect(buttons[0].text()).toBe('打开')
    expect(buttons[1].text()).toBe('复制链接')
  })

  it('有 items prop 时按 items 渲染按钮', () => {
    const wrapper = mountMenu({
      items: [
        { id: 'openLink', label: '在浏览器中打开' },
        { id: 'copyLink', label: '复制 URL' },
        { id: 'delete', label: '删除' },
      ],
    })

    const buttons = wrapper.findAll('button')
    expect(buttons).toHaveLength(3)
    expect(buttons[0].text()).toBe('在浏览器中打开')
    expect(buttons[1].text()).toBe('复制 URL')
    expect(buttons[2].text()).toBe('删除')
  })

  it('菜单定位在指定坐标', () => {
    const wrapper = mountMenu({ x: 150, y: 300 })

    const menu = wrapper.find('.context-menu')
    expect(menu.attributes('style')).toContain('left: 150px')
    expect(menu.attributes('style')).toContain('top: 300px')
  })

  it('role="menu" 用于无障碍', () => {
    const wrapper = mountMenu()

    expect(wrapper.find('.context-menu').attributes('role')).toBe('menu')
  })

  it('所有按钮 role="menuitem"', () => {
    const wrapper = mountMenu({
      items: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }],
    })

    wrapper.findAll('button').forEach(btn => {
      expect(btn.attributes('role')).toBe('menuitem')
    })
  })

  it('无 items 时按钮也有 menuitem role', () => {
    const wrapper = mountMenu()

    wrapper.findAll('button').forEach(btn => {
      expect(btn.attributes('role')).toBe('menuitem')
    })
  })
})

describe('ContextMenu.vue — 交互', () => {
  function mountMenu(props: Record<string, unknown> = {}) {
    return mount(ContextMenu, {
      props: { x: 100, y: 200, ...props },
    })
  }

  it('点击按钮 emit action', () => {
    const wrapper = mountMenu({
      items: [
        { id: 'openLink', label: '打开' },
        { id: 'copyLink', label: '复制' },
      ],
    })

    wrapper.findAll('button')[1].trigger('click')

    expect(wrapper.emitted('action')?.[0]).toEqual(['copyLink'])
  })

  it('无 items 时点击第一个按钮 emit open', () => {
    const wrapper = mountMenu()

    wrapper.findAll('button')[0].trigger('click')

    expect(wrapper.emitted('open')).toBeTruthy()
  })

  it('无 items 时点击第二个按钮 emit copy', () => {
    const wrapper = mountMenu()

    wrapper.findAll('button')[1].trigger('click')

    expect(wrapper.emitted('copy')).toBeTruthy()
  })
})

describe('ContextMenu.vue — 键盘导航', () => {
  function mountMenu(props: Record<string, unknown> = {}) {
    return mount(ContextMenu, {
      props: { x: 0, y: 0, ...props },
      attachTo: document.body,
    })
  }

  it('ArrowDown 聚焦下一个按钮（循环）', async () => {
    const wrapper = mountMenu({
      items: [
        { id: 'a', label: 'A' },
        { id: 'b', label: 'B' },
        { id: 'c', label: 'C' },
      ],
    })

    // 等待 onMounted 自动聚焦完成后再测试键盘导航
    await new Promise(resolve => setTimeout(resolve, 10))

    const buttons = wrapper.findAll('button')

    // 聚焦第一个，然后按 ArrowDown
    buttons[0].element.focus()
    await buttons[0].trigger('keydown', { key: 'ArrowDown' })
    expect(document.activeElement).toBe(buttons[1].element)

    // 再按 → 第三个
    await buttons[1].trigger('keydown', { key: 'ArrowDown' })
    expect(document.activeElement).toBe(buttons[2].element)

    // 再按 → 循环回第一个
    await buttons[2].trigger('keydown', { key: 'ArrowDown' })
    expect(document.activeElement).toBe(buttons[0].element)
  })

  it('ArrowUp 聚焦上一个按钮（循环）', async () => {
    const wrapper = mountMenu({
      items: [
        { id: 'a', label: 'A' },
        { id: 'b', label: 'B' },
      ],
    })

    // 等待 onMounted 自动聚焦完成后再测试键盘导航
    await new Promise(resolve => setTimeout(resolve, 10))

    const buttons = wrapper.findAll('button')

    buttons[0].element.focus()
    await buttons[0].trigger('keydown', { key: 'ArrowUp' })
    expect(document.activeElement).toBe(buttons[1].element)

    await buttons[1].trigger('keydown', { key: 'ArrowUp' })
    expect(document.activeElement).toBe(buttons[0].element)
  })

  it('Escape 调用 close emit', async () => {
    const wrapper = mountMenu({
      items: [{ id: 'a', label: 'A' }],
    })

    await wrapper.trigger('keydown', { key: 'Escape' })

    expect(wrapper.emitted('close')).toBeTruthy()
  })
})

describe('ContextMenu.vue — 聚焦行为', () => {
  it('挂载后自动聚焦第一个按钮', async () => {
    const wrapper = mount(ContextMenu, {
      props: {
        x: 0,
        y: 0,
        items: [
          { id: 'a', label: 'A' },
          { id: 'b', label: 'B' },
        ],
      },
      attachTo: document.body,
    })

    // onMounted → nextTick → 聚焦第一个 button
    await new Promise(resolve => requestAnimationFrame(resolve))
    await new Promise(resolve => setTimeout(resolve, 10))

    const buttons = wrapper.findAll('button')
    expect(document.activeElement).toBe(buttons[0].element)
  })
})
