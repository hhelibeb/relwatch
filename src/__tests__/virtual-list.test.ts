import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import VirtualList from '../components/common/VirtualList.vue'

// 简单虚拟列表的真实运行场景测试
// - 小列表（≤ virtualizeThreshold）全量渲染，不引入虚拟化
// - 大列表只渲染可视区附近的项，容器高度等于总估算高度
// - itemKey 写入 data-vkey 用于行高测量

interface TestItem {
  id: number
}

function makeItems(n: number): TestItem[] {
  return Array.from({ length: n }, (_, i) => ({ id: i + 1 }))
}

function mountList(items: TestItem[]) {
  return mount(VirtualList as any, {
    props: {
      items,
      itemKey: (item: TestItem) => item.id,
      estimatedHeight: 120,
      gap: 8,
    },
    slots: {
      default: `<div class="row">{{ item.id }}</div>`,
    },
  })
}

describe('VirtualList.vue', () => {
  it('小列表（小于阈值）全量渲染所有项', () => {
    const wrapper = mountList(makeItems(10))

    expect(wrapper.findAll('.row')).toHaveLength(10)
    expect(wrapper.find('.virtual-list').exists()).toBe(false)
    expect(wrapper.find('.virtual-list-plain').exists()).toBe(true)
  })

  it('大列表启用虚拟化，仅渲染可视区附近的项', async () => {
    const wrapper = mountList(makeItems(300))

    // 容器为虚拟滚动结构（非全量渲染），且高度等于总估算高度
    expect(wrapper.find('.virtual-list').exists()).toBe(true)
    const rendered = wrapper.findAll('.virtual-item')
    expect(rendered.length).toBeGreaterThan(0)
    expect(rendered.length).toBeLessThan(20)

    // 总高度按 300 行 * (120 + 8) 估算
    const height = wrapper.find('.virtual-list').attributes('style')
    expect(height).toContain('38400px')

    // 每个可见行都带 data-vkey，用于行高测量
    const first = rendered[0]
    expect(first.attributes('data-vkey')).toBeTruthy()
    expect(first.find('.row').text()).toBeTruthy()
  })

  it('空列表渲染空内容（不进入虚拟化分支）', () => {
    const wrapper = mountList([])

    expect(wrapper.find('.virtual-list').exists()).toBe(false)
    expect(wrapper.findAll('.row')).toHaveLength(0)
  })
})

describe('VirtualList.vue — 行高测量响应式', () => {
  const originalDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!

  afterEach(() => {
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', originalDescriptor)
  })

  // jsdom 的 offsetHeight 恒为 0；按 data-vkey 模拟真实布局高度（奇数行 300，偶数行 120）
  function stubOffsetHeight() {
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get() {
        const el = this as HTMLElement
        const key = el.getAttribute?.('data-vkey')
        if (key) return Number(key) % 2 === 1 ? 300 : 120
        return 0
      },
    })
  }

  it('测量写回后触发重算：容器总高度按真实行高更新', async () => {
    stubOffsetHeight()
    const wrapper = mount(VirtualList as any, {
      props: {
        items: makeItems(300),
        itemKey: (item: TestItem) => item.id,
        estimatedHeight: 120,
        gap: 8,
      },
      slots: {
        default: `<div class="row">{{ item.id }}</div>`,
      },
    })

    // 测量前：全按估算 300 * (120 + 8) = 38400
    expect(wrapper.find('.virtual-list').attributes('style')).toContain('38400px')

    await flushPromises()

    // 首屏可见行（id 1/3 等奇数行）测量为 300，总高度应随测量更新
    // 1,3 两行各 +180，故总高度 = 38400 + 360 = 38760
    expect(wrapper.find('.virtual-list').attributes('style')).toContain('38760px')
  })
})

describe('VirtualList.vue — scrollToIndex（通知定位）', () => {
  let host: HTMLElement | null = null
  let mounted: ReturnType<typeof mount> | null = null
  let scrollTop = 0

  // 挂到真实的滚动容器下：findScrollParent 依赖 computed overflowY，
  // jsdom 无布局故用自定义属性接管 scrollTop 以便断言
  function mountInScroller(items: TestItem[]) {
    host = document.createElement('div')
    host.style.overflowY = 'auto'
    document.body.appendChild(host)
    scrollTop = 0
    Object.defineProperty(host, 'scrollTop', {
      configurable: true,
      get: () => scrollTop,
      set: (v: number) => {
        scrollTop = v
      },
    })
    mounted = mount(VirtualList as any, {
      props: {
        items,
        itemKey: (item: TestItem) => item.id,
        estimatedHeight: 120,
        gap: 8,
      },
      slots: { default: `<div class="row">{{ item.id }}</div>` },
      attachTo: host,
    })
    return mounted
  }

  afterEach(() => {
    // 必须卸载：残留的 rAF 校正回调会继续写共享的 scrollTop，污染后续用例
    mounted?.unmount()
    mounted = null
    host?.remove()
    host = null
  })

  it('非虚拟化：滚动到对应下标的真实 DOM 行', () => {
    const wrapper = mountInScroller(makeItems(10))
    const rows = wrapper.findAll('.row')
    const spy = vi.fn()
    ;(rows[5].element as unknown as { scrollIntoView: unknown }).scrollIntoView = spy

    ;(wrapper.vm as unknown as { scrollToIndex: (i: number) => void }).scrollToIndex(5)

    // 命中第 6 行（id=6），且用 nearest 避免整页跳动
    expect(spy).toHaveBeenCalledWith({ block: 'nearest' })
    expect(rows[5].text()).toBe('6')
  })

  it('下标越界时不滚动也不抛错', () => {
    const wrapper = mountInScroller(makeItems(10))
    const rows = wrapper.findAll('.row')
    const spy = vi.fn()
    for (const row of rows) {
      ;(row.element as unknown as { scrollIntoView: unknown }).scrollIntoView = spy
    }

    const vm = wrapper.vm as unknown as { scrollToIndex: (i: number) => void }
    expect(() => vm.scrollToIndex(10)).not.toThrow()
    expect(() => vm.scrollToIndex(-1)).not.toThrow()
    expect(spy).not.toHaveBeenCalled()
  })

  it('虚拟化：目标不在可视区时把滚动容器定位到估算位置', async () => {
    const wrapper = mountInScroller(makeItems(300))
    await flushPromises()

    ;(wrapper.vm as unknown as { scrollToIndex: (i: number) => void }).scrollToIndex(200)

    // 200 行 * (120 + 8) = 25600，减去顶部留白 12（jsdom 无布局：视口 0、无 sticky）
    expect(scrollTop).toBe(200 * 128 - 12)
  })

  it('虚拟化：为顶部 sticky 栏预留空间（目标行不被搜索框遮住）', async () => {
    const wrapper = mountInScroller(makeItems(300))
    await flushPromises()
    // 顶部 sticky 搜索/筛选栏：jsdom 无布局，需显式给出高度
    const bar = document.createElement('div')
    bar.style.position = 'sticky'
    bar.style.top = '0px'
    Object.defineProperty(bar, 'offsetHeight', { configurable: true, value: 56 })
    host!.insertBefore(bar, host!.firstChild)

    ;(wrapper.vm as unknown as { scrollToIndex: (i: number) => void }).scrollToIndex(200)

    // 估算位置 25600 − 留白 12 − sticky 栏 56
    expect(scrollTop).toBe(200 * 128 - 12 - 56)
  })

  it('虚拟化：已挂载的目标行校正到 sticky 栏下方并居中（不按容器偏移计算增量）', async () => {
    const wrapper = mountInScroller(makeItems(300))
    await flushPromises()
    // 视口 600、顶部 sticky 栏 56
    Object.defineProperty(host!, 'clientHeight', { configurable: true, value: 600 })
    const bar = document.createElement('div')
    bar.style.position = 'sticky'
    bar.style.top = '0px'
    Object.defineProperty(bar, 'offsetHeight', { configurable: true, value: 56 })
    host!.insertBefore(bar, host!.firstChild)
    // 目标行（index 1）当前位于视口 y=400，行高 300
    const row = wrapper.findAll('.virtual-item').find(v => v.attributes('data-vkey') === '2')!.element as HTMLElement
    Object.defineProperty(row, 'offsetHeight', { configurable: true, value: 300 })
    row.getBoundingClientRect = () => ({ top: 400 }) as DOMRect

    ;(wrapper.vm as unknown as { scrollToIndex: (i: number) => void }).scrollToIndex(1)

    // 目标行顶部应落在 sticky 下沿(56) + 剩余可用区居中留白((600-56-300)/2 = 122)
    // 即目标 y = 178；当前 y = 400 → 需向下滚 222
    expect(scrollTop).toBe(222)
  })

  it('虚拟化：目标已在可视区时不动滚动位置（顶部未被 sticky 顶出负值）', async () => {
    const wrapper = mountInScroller(makeItems(300))
    await flushPromises()
    const bar = document.createElement('div')
    bar.style.position = 'sticky'
    bar.style.top = '0px'
    Object.defineProperty(bar, 'offsetHeight', { configurable: true, value: 56 })
    host!.insertBefore(bar, host!.firstChild)

    ;(wrapper.vm as unknown as { scrollToIndex: (i: number) => void }).scrollToIndex(0)

    // 第 0 行无法再往上让位：scrollTop 夹到 0，不出现负值
    expect(scrollTop).toBe(0)
  })
})

describe('VirtualList.vue — v-show 隐藏容器显示后行高重测', () => {
  const originalDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!
  const originalRO = (globalThis as any).ResizeObserver
  let roInstances: { cb: ResizeObserverCallback; targets: Element[] }[] = []
  let host: HTMLElement | null = null

  beforeEach(() => {
    roInstances = []
    // jsdom 无 ResizeObserver；mock 捕获实例，手动触发回调模拟 v-show 显示切换
    class MockResizeObserver {
      cb: ResizeObserverCallback
      targets: Element[] = []
      constructor(cb: ResizeObserverCallback) {
        this.cb = cb
        roInstances.push(this)
      }
      observe(target: Element) { this.targets.push(target) }
      unobserve() {}
      disconnect() {}
    }
    ;(globalThis as any).ResizeObserver = MockResizeObserver
  })

  afterEach(() => {
    if (originalRO) (globalThis as any).ResizeObserver = originalRO
    else delete (globalThis as any).ResizeObserver
    host?.remove()
    host = null
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', originalDescriptor)
  })

  function fireResize() {
    for (const inst of roInstances) inst.cb([], inst as any)
  }

  it('隐藏时挂载行未测量（offsetHeight=0），显示后重测收敛', async () => {
    // 阶段 1：容器隐藏（display:none），offsetHeight 恒 0 → 行高测量全部跳过
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get() { return 0 },
    })
    host = document.createElement('div')
    host.style.overflowY = 'auto'
    document.body.appendChild(host)
    const wrapper = mount(VirtualList as any, {
      props: {
        items: makeItems(300),
        itemKey: (item: TestItem) => item.id,
        estimatedHeight: 120,
        gap: 8,
      },
      slots: { default: `<div class="row">{{ item.id }}</div>` },
      attachTo: host,
    })
    await flushPromises()

    // 隐藏期间已渲染，但全部回退估计高度：容器总高 300 * (120+8) = 38400
    expect(wrapper.find('.virtual-list').attributes('style')).toContain('38400px')
    expect(wrapper.findAll('.virtual-item').length).toBeGreaterThan(0)

    // 阶段 2：容器变为可见，offsetHeight 恢复真实布局（奇数 300 / 偶数 120）
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
      configurable: true,
      get() {
        const el = this as HTMLElement
        const key = el.getAttribute?.('data-vkey')
        if (key) return Number(key) % 2 === 1 ? 300 : 120
        return 1 // 容器本身可见（非 0）
      },
    })

    // 模拟 v-show 从隐藏变为可见 → 观察容器自身的 ResizeObserver 触发重测
    fireResize()
    await flushPromises()

    // 首屏行（id 1/2/3）测出真实高度后总高收敛：38400 + (300-120) * 2 = 38760
    expect(wrapper.find('.virtual-list').attributes('style')).toContain('38760px')
  })
})
