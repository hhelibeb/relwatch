import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { defineComponent } from 'vue'
import { mount } from '@vue/test-utils'
import {
  registerCloser,
  unregisterCloser,
  isTransientUiInvalidated,
  endTransientUiInvalidation,
} from '../composables/contextMenuBus'
import { useTransientUiGuard } from '../composables/useTransientUiGuard'

/**
 * useTransientUiGuard — 窗口隐藏/失焦后的瞬态 UI 守卫
 *
 * 复现来源：悬浮摘要 → 左键点一下（元素获得焦点）→ 鼠标移开 → 关到托盘 →
 * 托盘图标重新打开。实测 Chromium 在隐藏时只补 blur/visibilitychange，
 * 重新显示时重放隐藏前的 focus，导致摘要提示自己冒出来。
 */

type FocusHandler = (event: { payload: boolean }) => void

const unlisten = vi.fn()
const onFocusChanged = vi.fn(async (_handler: FocusHandler) => unlisten)
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged }),
}))

// 用真实 contextMenuBus（模块级单例）：每个用例自己收尾
const closers: (() => void)[] = []
const mounted: { unmount: () => void }[] = []

function mountGuard(onInvalidate?: () => void) {
  const Host = defineComponent({
    setup() {
      useTransientUiGuard({ onInvalidate })
      return () => null
    },
  })
  // 必须每例卸载：监听器挂在 window/document 上，残留会跨例堆叠事件与 closer 调用
  const wrapper = mount(Host)
  mounted.push(wrapper)
  return wrapper
}

function trackCloser() {
  const closer = vi.fn()
  registerCloser(closer)
  closers.push(closer)
  return closer
}

function moveMouse(clientX: number, clientY: number) {
  window.dispatchEvent(new MouseEvent('mousemove', { clientX, clientY }))
}

/** 显式控制 document.hidden 再派发 visibilitychange（不依赖 jsdom 默认值） */
function withDocumentHidden(hidden: boolean, fn: () => void) {
  const original = Object.getOwnPropertyDescriptor(document, 'hidden')
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden })
  try {
    fn()
  } finally {
    if (original) Object.defineProperty(document, 'hidden', original)
    else delete (document as unknown as Record<string, unknown>).hidden
  }
}

beforeEach(() => {
  endTransientUiInvalidation()
  onFocusChanged.mockClear()
  unlisten.mockClear()
})

afterEach(() => {
  // 模块级单例：每个用例结束都要卸载守卫、解除失效期并撤掉自己注册的 closer
  for (const w of mounted.splice(0)) w.unmount()
  endTransientUiInvalidation()
  for (const c of closers.splice(0)) unregisterCloser(c)
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('useTransientUiGuard — 隐藏/失焦 → 失效', () => {
  it('窗口失焦（blur）：关闭瞬态层并进入失效期', async () => {
    const closer = trackCloser()
    const onInvalidate = vi.fn()
    mountGuard(onInvalidate)

    expect(isTransientUiInvalidated()).toBe(false)
    window.dispatchEvent(new Event('blur'))

    expect(closer).toHaveBeenCalledOnce()
    expect(onInvalidate).toHaveBeenCalledOnce()
    expect(isTransientUiInvalidated()).toBe(true)
  })

  it('页面转为隐藏（visibilitychange hidden=true）：同样失效', () => {
    const closer = trackCloser()
    mountGuard()

    withDocumentHidden(true, () => document.dispatchEvent(new Event('visibilitychange')))

    expect(closer).toHaveBeenCalledOnce()
    expect(isTransientUiInvalidated()).toBe(true)
  })

  it('页面保持可见的 visibilitychange 不触发失效', () => {
    const closer = trackCloser()
    mountGuard()

    withDocumentHidden(false, () => document.dispatchEvent(new Event('visibilitychange')))

    expect(closer).not.toHaveBeenCalled()
    expect(isTransientUiInvalidated()).toBe(false)
  })

  it('Tauri 窗口焦点变化（失焦）也触发失效，卸载时注销监听', async () => {
    const closer = trackCloser()
    ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
    const wrapper = mountGuard()
    await Promise.resolve()
    await Promise.resolve()

    expect(onFocusChanged).toHaveBeenCalledOnce()
    const handler = onFocusChanged.mock.calls[0]![0]
    handler({ payload: false })
    expect(closer).toHaveBeenCalledOnce()
    expect(isTransientUiInvalidated()).toBe(true)

    // 重新获得焦点不触发失效；卸载后不再监听
    endTransientUiInvalidation()
    handler({ payload: true })
    expect(isTransientUiInvalidated()).toBe(false)

    wrapper.unmount()
    await Promise.resolve()
    expect(unlisten).toHaveBeenCalledOnce()
  })
})

describe('useTransientUiGuard — 真实用户输入 → 解除失效', () => {
  it('鼠标移动到新坐标才算用户回来了', () => {
    mountGuard()
    // 隐藏前最后位置：卡摘要上
    moveMouse(120, 300)
    window.dispatchEvent(new Event('blur'))
    expect(isTransientUiInvalidated()).toBe(true)

    // 窗口恢复时 Chromium 用旧坐标重放的事件：不放行
    moveMouse(120, 300)
    expect(isTransientUiInvalidated()).toBe(true)

    // 真实的移动：解除
    moveMouse(400, 500)
    expect(isTransientUiInvalidated()).toBe(false)
  })

  it('从未收到过指针事件时（无基准坐标）首个移动即解除', () => {
    mountGuard()
    window.dispatchEvent(new Event('blur'))
    expect(isTransientUiInvalidated()).toBe(true)

    moveMouse(400, 500)
    expect(isTransientUiInvalidated()).toBe(false)
  })

  it('点击 / 按键 / 滚轮无条件解除（这些事件不会被重放）', () => {
    mountGuard()
    for (const event of ['mousedown', 'pointerdown', 'keydown', 'wheel']) {
      window.dispatchEvent(new Event('blur'))
      expect(isTransientUiInvalidated()).toBe(true)

      window.dispatchEvent(new Event(event, { bubbles: true }))
      expect(isTransientUiInvalidated(), `${event} 应解除失效期`).toBe(false)
    }
  })

  it('未失焦时鼠标移动不产生任何副作用', () => {
    const closer = trackCloser()
    mountGuard()
    moveMouse(10, 10)
    moveMouse(20, 20)
    expect(closer).not.toHaveBeenCalled()
    expect(isTransientUiInvalidated()).toBe(false)
  })
})

describe('useTransientUiGuard — 卸载清理', () => {
  it('卸载后 blur 不再关闭瞬态层，且不把失效期留在全局', () => {
    const closer = trackCloser()
    const wrapper = mountGuard()
    window.dispatchEvent(new Event('blur'))
    expect(closer).toHaveBeenCalledOnce()

    wrapper.unmount()
    expect(isTransientUiInvalidated()).toBe(false)

    window.dispatchEvent(new Event('blur'))
    expect(closer).toHaveBeenCalledOnce() // 仍是 1 次：监听已摘掉
  })
})
