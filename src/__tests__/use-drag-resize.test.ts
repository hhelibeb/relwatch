import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { defineComponent, ref, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { useDragResize } from '../composables/useDragResize'

const KEY = 'relwatch.test.drag-resize'
// 弹窗默认尺寸 760x560，jsdom 视口 1024x768 下居中基座为 (132, 104)
const DEFAULT_W = 760
const DEFAULT_H = 560

const Harness = defineComponent({
  setup() {
    const el = ref<HTMLElement | null>(null)
    const { startDrag, startResize } = useDragResize(el, {
      minWidth: 400,
      minHeight: 300,
      persistKey: KEY,
    })
    return { el, startDrag, startResize }
  },
  template: `
    <div ref="el" class="modal">
      <header class="drag-zone" @pointerdown="startDrag">
        header
        <button class="inner-btn">x</button>
      </header>
      <div class="handle-se" @pointerdown="startResize($event, 'se')"></div>
      <div class="handle-nw" @pointerdown="startResize($event, 'nw')"></div>
    </div>
  `,
})

// jsdom 无布局引擎，getBoundingClientRect 恒为 0。
// 这里模拟「flex 居中基座 + transform 偏移 + 内联尺寸」的真实行为：
// 基座随尺寸居中，rect 随 transform 平移。
function mockLayout(el: HTMLElement) {
  vi.spyOn(el, 'getBoundingClientRect').mockImplementation(() => {
    const w = el.style.width ? parseFloat(el.style.width) : DEFAULT_W
    const h = el.style.height ? parseFloat(el.style.height) : DEFAULT_H
    const m = /translate\((-?\d+(?:\.\d+)?)px,\s*(-?\d+(?:\.\d+)?)px\)/.exec(el.style.transform || '')
    const dx = m ? parseFloat(m[1]) : 0
    const dy = m ? parseFloat(m[2]) : 0
    const left = (window.innerWidth - w) / 2 + dx
    const top = (window.innerHeight - h) / 2 + dy
    return {
      left,
      top,
      width: w,
      height: h,
      right: left + w,
      bottom: top + h,
      x: left,
      y: top,
      toJSON: () => ({}),
    } as DOMRect
  })
}

function pointer(target: EventTarget, type: string, init: MouseEventInit = {}) {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, ...init }))
}

function mountHarness() {
  const wrapper = mount(Harness, { attachTo: document.body })
  const el = wrapper.find('.modal').element as HTMLElement
  mockLayout(el)
  return { wrapper, el }
}

beforeEach(() => {
  window.localStorage.clear()
})

afterEach(() => {
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('useDragResize', () => {
  it('拖动标题栏移动弹窗，结束时持久化位置（不含尺寸）', async () => {
    const { wrapper, el } = mountHarness()

    pointer(wrapper.find('.drag-zone').element, 'pointerdown', { button: 0, clientX: 200, clientY: 120 })
    pointer(window, 'pointermove', { buttons: 1, clientX: 250, clientY: 160 })
    expect(el.style.transform).toBe('translate(50px, 40px)')
    pointer(window, 'pointerup', { buttons: 0 })

    // 132+50=182, 104+40=144；未调整尺寸则不持久化 w/h，保留 CSS 默认响应式宽度
    expect(window.localStorage.getItem(KEY)).toBe(JSON.stringify({ x: 182, y: 144 }))
    wrapper.unmount()
  })

  it('拖动被钳制在视口内', () => {
    const { wrapper, el } = mountHarness()

    pointer(wrapper.find('.drag-zone').element, 'pointerdown', { button: 0, clientX: 200, clientY: 120 })
    pointer(window, 'pointermove', { buttons: 1, clientX: -9999, clientY: 9999 })
    // left 钳到 0 → 偏移 0-132=-132；top 钳到 768-560=208 → 偏移 208-104=104
    expect(el.style.transform).toBe('translate(-132px, 104px)')
    pointer(window, 'pointerup', { buttons: 0 })
    wrapper.unmount()
  })

  it('标题栏内按钮上不触发拖动', () => {
    const { wrapper, el } = mountHarness()

    pointer(wrapper.find('.inner-btn').element, 'pointerdown', { button: 0, clientX: 200, clientY: 120 })
    pointer(window, 'pointermove', { buttons: 1, clientX: 260, clientY: 180 })
    expect(el.style.transform).toBe('')
    pointer(window, 'pointerup', { buttons: 0 })
    wrapper.unmount()
  })

  it('se 手柄调整大小：受最小尺寸约束，解除 max-height，结束时持久化尺寸', () => {
    const { wrapper, el } = mountHarness()

    pointer(wrapper.find('.handle-se').element, 'pointerdown', { button: 0, clientX: 900, clientY: 700 })
    pointer(window, 'pointermove', { buttons: 1, clientX: 200, clientY: 200 })
    // w = clamp(200-132, 400, 1024-132) = 400；h = clamp(200-104, 300, 768-104) = 300
    expect(el.style.width).toBe('400px')
    expect(el.style.height).toBe('300px')
    expect(el.style.maxHeight).toBe('none')
    // 左/上边缘固定：偏移 = 132-(1024-400)/2=-180, 104-(768-300)/2=-130
    expect(el.style.transform).toBe('translate(-180px, -130px)')
    pointer(window, 'pointerup', { buttons: 0 })

    expect(window.localStorage.getItem(KEY)).toBe(JSON.stringify({ x: 132, y: 104, w: 400, h: 300 }))
    wrapper.unmount()
  })

  it('nw 手柄调整大小：右/下边缘固定', () => {
    const { wrapper, el } = mountHarness()

    pointer(wrapper.find('.handle-nw').element, 'pointerdown', { button: 0, clientX: 132, clientY: 104 })
    pointer(window, 'pointermove', { buttons: 1, clientX: 100, clientY: 80 })
    // w = 132+760-100=792；h = 104+560-80=584
    expect(el.style.width).toBe('792px')
    expect(el.style.height).toBe('584px')
    // 右缘固定 left=892-792=100 → 偏移 100-(1024-792)/2=-16；下缘固定 top=664-584=80 → 偏移 80-(768-584)/2=-12
    expect(el.style.transform).toBe('translate(-16px, -12px)')
    pointer(window, 'pointerup', { buttons: 0 })
    wrapper.unmount()
  })

  it('下次挂载时从 localStorage 恢复尺寸与位置', async () => {
    window.localStorage.setItem(KEY, JSON.stringify({ x: 10, y: 20, w: 500, h: 400 }))
    const { wrapper, el } = mountHarness()
    await nextTick()

    expect(el.style.width).toBe('500px')
    expect(el.style.height).toBe('400px')
    expect(el.style.maxHeight).toBe('none')
    // 偏移 = 10-(1024-500)/2=-252, 20-(768-400)/2=-164
    expect(el.style.transform).toBe('translate(-252px, -164px)')
    wrapper.unmount()
  })

  it('持久化数据非法时静默忽略，不破坏默认布局', async () => {
    window.localStorage.setItem(KEY, 'not-json{{{')
    const { wrapper, el } = mountHarness()
    await nextTick()

    expect(el.style.transform).toBe('')
    expect(el.style.width).toBe('')
    wrapper.unmount()
  })
})
