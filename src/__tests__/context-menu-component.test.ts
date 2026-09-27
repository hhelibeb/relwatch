import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { defineComponent, ref, onUnmounted } from 'vue'
import ContextMenu from '../components/common/ContextMenu.vue'
import { registerCloser, unregisterCloser, closeAllContextMenus } from '../composables/contextMenuBus'

beforeEach(() => {
  vi.clearAllMocks()
})

/**
 * ContextMenu.vue 真实运行场景测试
 *
 * 固定定位右键菜单，支持：
 * - 按 items 渲染按钮（items 为必填 prop）
 * - 挂载后自动聚焦第一个按钮
 * - 键盘导航：ArrowDown/ArrowUp（循环）、Escape → close
 */
describe('ContextMenu.vue — 渲染', () => {
  function mountMenu(props: Record<string, unknown> = {}) {
    return mount(ContextMenu, {
      props: { x: 100, y: 200, items: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }], ...props },
    })
  }

  it('按 items 渲染按钮', () => {
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
})

describe('ContextMenu.vue — 交互', () => {
  function mountMenu(props: Record<string, unknown> = {}) {
    return mount(ContextMenu, {
      props: { x: 100, y: 200, items: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }], ...props },
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
})

describe('ContextMenu.vue — 键盘导航', () => {
  function mountMenu(props: Record<string, unknown> = {}) {
    return mount(ContextMenu, {
      props: { x: 0, y: 0, items: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }], ...props },
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

/**
 * 复用同一实例重开菜单：宿主（ReleaseItem/SourceTab/App 等）的真实写法是
 * 「先 closeAllContextMenus() 关掉旧菜单，再在同一 tick 内写入新坐标」。
 * Vue 把这两次状态变更批处理成一次 patch，v-if 前后皆为真 → 组件实例被复用，
 * onMounted 不会再跑。落点若只在 onMounted 里算，第二次右键就会停在上一处位置
 * （右键视频封面后再右键跳转链接按钮即复现）。
 */
describe('ContextMenu.vue — 复用实例重开', () => {
  const Host = defineComponent({
    components: { ContextMenu },
    props: { items: { type: Array, required: true } },
    setup() {
      const menu = ref<{ x: number; y: number } | null>(null)
      // 与真实宿主一致：注册 closer 供总线互斥关闭，卸载时注销
      const closer = () => { menu.value = null }
      registerCloser(closer)
      onUnmounted(() => unregisterCloser(closer))
      function open(x: number, y: number) {
        closeAllContextMenus() // 真实链路：总线关闭全部已注册菜单
        menu.value = { x, y }
      }
      return { menu, open }
    },
    template: `<div><ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="items" /></div>`,
  })

  type HostVm = { open: (x: number, y: number) => void }
  let wrapper: VueWrapper | null = null

  function mountHost(items: { id: string; label: string }[]) {
    wrapper = mount(Host, { props: { items }, attachTo: document.body })
    return wrapper
  }

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
  })

  it('第二次右键落在新锚点，而不是上一处落点', async () => {
    const w = mountHost([{ id: 'a', label: 'A' }])
    const vm = w.vm as unknown as HostVm

    vm.open(100, 100)
    await flushPromises()
    const firstEl = w.find('.context-menu').element
    expect(w.find('.context-menu').attributes('style')).toContain('left: 100px')

    vm.open(500, 500)
    await flushPromises()
    const secondEl = w.find('.context-menu').element

    // 确证走的是实例复用（patch）路径，而非重新挂载——这正是原缺陷的触发条件
    expect(secondEl).toBe(firstEl)
    expect(w.find('.context-menu').attributes('style')).toContain('left: 500px')
    expect(w.find('.context-menu').attributes('style')).toContain('top: 500px')
  })

  it('复用重开时视口钳位一并重算（jsdom 无视口尺寸，用 rect stub 触发越界）', async () => {
    const w = mountHost([{ id: 'a', label: 'A' }])
    const vm = w.vm as unknown as HostVm

    vm.open(900, 700)
    await flushPromises()
    // jsdom 无布局引擎，rect 恒为 0 → 首开不钳位
    expect(w.find('.context-menu').attributes('style')).toContain('left: 900px')

    // 菜单真实尺寸 300×400，视口 1024×768 → 右下角越界
    vi.spyOn(w.find('.context-menu').element, 'getBoundingClientRect').mockReturnValue(
      { width: 300, height: 400 } as DOMRect,
    )
    vm.open(950, 750)
    await flushPromises()

    // left = 1024-300-4 = 720；top = 768-400-4 = 364
    expect(w.find('.context-menu').attributes('style')).toContain('left: 720px')
    expect(w.find('.context-menu').attributes('style')).toContain('top: 364px')
  })

  it('菜单项变化导致高度变化时也重算钳位', async () => {
    const w = mountHost([{ id: 'a', label: 'A' }])
    const vm = w.vm as unknown as HostVm

    vm.open(900, 700)
    await flushPromises()
    expect(w.find('.context-menu').attributes('style')).toContain('top: 700px')

    vi.spyOn(w.find('.context-menu').element, 'getBoundingClientRect').mockReturnValue(
      { width: 300, height: 400 } as DOMRect,
    )
    // 封面菜单比链接菜单多 3 项（菜单变高），同一实例内换项后钳位需跟着重算
    await w.setProps({ items: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }] })
    await flushPromises()

    expect(w.find('.context-menu').attributes('style')).toContain('left: 720px')
    expect(w.find('.context-menu').attributes('style')).toContain('top: 364px')
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
