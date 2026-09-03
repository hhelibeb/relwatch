import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import { hasActiveOverlay } from '../composables/contextMenuBus'
import { useAnchoredMenu } from '../components/agent/useAnchoredMenu'

// 挂到 document 上：Esc 后焦点回归锚点的断言依赖元素已连接（jsdom 中未连接元素 focus() 无效）
// 在宿主组件 setup 内调用 composable（onUnmounted/watch 需要组件实例）
const wrappers: { unmount: () => void }[] = []
function setupWith<T>(useFn: () => T) {
  let api!: T
  const host = defineComponent({
    setup() {
      api = useFn()
      return {}
    },
    template: '<div/>',
  })
  const wrapper = mount(host, { attachTo: document.body })
  wrappers.push(wrapper)
  return { wrapper, api }
}

function makeAnchor(rect: { left: number; right: number; top: number; bottom: number }): HTMLElement {
  const el = document.createElement('button')
  document.body.appendChild(el)
  const r = { width: rect.right - rect.left, height: rect.bottom - rect.top, x: rect.left, y: rect.top, ...rect, toJSON: () => rect }
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue(r as DOMRect)
  return el
}

beforeEach(() => {
  localStorage.clear()
})
afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount()
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('useAnchoredMenu 定位（对齐既有两处实现的语义）', () => {
  it('右对齐锚点右缘（会话 ⋯ 菜单：width 148，超出视口时钳制）', () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose: () => (open.value = false) }),
    )
    const anchor = makeAnchor({ left: 900, right: 940, top: 6, bottom: 28 })
    api.place(anchor)
    // x = max(8, min(right - 148, innerWidth - 148)) = min(792, 876) = 792；y = bottom + 4
    expect(api.pos.value).toEqual({ x: 792, y: 32 })
  })

  it('右对齐且锚点太靠右时钳制到视口右缘减宽度', () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose: () => (open.value = false) }),
    )
    api.place(makeAnchor({ left: 990, right: 1030, top: 0, bottom: 20 }))
    // min(1030-148=882, 1024-148=876) → 876
    expect(api.pos.value.x).toBe(876)
  })

  it('左对齐锚点左缘（rpc 状态菜单：width 216）', () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 216, align: 'left', isOpen: open, onClose: () => (open.value = false) }),
    )
    api.place(makeAnchor({ left: 60, right: 78, top: 4, bottom: 26 }))
    // x = max(8, min(60, 1024-216)) = 60；y = 30
    expect(api.pos.value).toEqual({ x: 60, y: 30 })
  })

  it('左对齐且空间不足时钳制（面板贴右缘：菜单右缘贴视口右缘）', () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 216, align: 'left', isOpen: open, onClose: () => (open.value = false) }),
    )
    api.place(makeAnchor({ left: 990, right: 1008, top: 0, bottom: 20 }))
    expect(api.pos.value.x).toBe(1024 - 216)
  })

  it('rect 取不到时保持原位置（与既有实现的 if (rect) 分支一致）', () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose: () => (open.value = false) }),
    )
    api.place(makeAnchor({ left: 100, right: 140, top: 0, bottom: 20 }))
    api.place(null)
    expect(api.pos.value.x).toBeGreaterThan(0) // 未被重置为 0
  })
})

describe('useAnchoredMenu B1：覆盖层注册 + document 级 Esc 关闭', () => {
  it('打开期间注册覆盖层（hasActiveOverlay 命中，Esc 不误触托盘最小化的前提）', async () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose: () => (open.value = false) }),
    )
    api.place(makeAnchor({ left: 0, right: 20, top: 0, bottom: 20 }))
    expect(hasActiveOverlay()).toBe(false)
    open.value = true
    await nextTick()
    expect(hasActiveOverlay()).toBe(true)
    open.value = false
    await nextTick()
    expect(hasActiveOverlay()).toBe(false)
  })

  it('打开时挂 document keydown 监听，Esc 触发 onClose 并让焦点回归锚点', async () => {
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose: () => (open.value = false) }),
    )
    const anchor = makeAnchor({ left: 0, right: 20, top: 0, bottom: 20 })
    api.place(anchor)
    open.value = true
    await nextTick()

    // 非 Escape 键不触发
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(open.value).toBe(true)

    anchor.focus()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(open.value).toBe(false)
    // 焦点回归触发按钮（对齐 useDropdown 的焦点管理习惯）
    expect(document.activeElement).toBe(anchor)
  })

  it('未打开时 Esc 不触发 onClose（监听虽在也按 isOpen 判空）', async () => {
    const onClose = vi.fn()
    const open = ref(false)
    setupWith(() => useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose }))
    open.value = true
    await nextTick()
    open.value = false
    await nextTick()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(onClose).not.toHaveBeenCalled()
  })

  it('关闭/卸载后注销监听与覆盖层（卸载后 Esc 不再触发，不泄漏）', async () => {
    const onClose = vi.fn()
    const open = ref(false)
    const { api } = setupWith(() =>
      useAnchoredMenu({ width: 148, align: 'right', isOpen: open, onClose }),
    )
    api.place(makeAnchor({ left: 0, right: 20, top: 0, bottom: 20 }))
    open.value = true
    await nextTick()
    expect(hasActiveOverlay()).toBe(true)

    wrappers[0]?.unmount()
    expect(hasActiveOverlay()).toBe(false)
    // 卸载后菜单仍处于"打开值"（调用方未复位），Esc 也不应再触发 onClose
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(onClose).not.toHaveBeenCalled()
  })
})
