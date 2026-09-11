import { describe, it, expect, vi, afterEach } from 'vitest'
import { defineComponent, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { useEscapeToTray } from '../composables/useEscapeToTray'
import { registerOverlayActive } from '../composables/contextMenuBus'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const invokeMock = vi.mocked(invoke)

const Harness = defineComponent({
  props: {
    enabled: {
      type: Boolean,
      default: true,
    },
  },
  setup(props) {
    useEscapeToTray(ref(props.enabled))
    return {}
  },
  template: '<div />',
})

// 统一登记 wrapper：断言失败时测试会在 unmount 前中断，遗留的 document 级监听
// 会污染后续用例（表现为 invoke 调用次数逐条累加）。这里在 afterEach 兜底卸载。
const mounted: ReturnType<typeof mount>[] = []
function mountHarness(props: { enabled?: boolean } = {}) {
  const w = mount(Harness, { props })
  mounted.push(w)
  return w
}

function pressEscape(target: EventTarget = document, init: KeyboardEventInit = {}) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, ...init }))
}

/** 在 body 下挂一个元素并聚焦，返回它（便于断言失焦后的 activeElement） */
function mountFocused<T extends HTMLElement>(el: T): T {
  document.body.appendChild(el)
  el.focus()
  return el
}

afterEach(() => {
  for (const w of mounted.splice(0)) w.unmount()
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

describe('useEscapeToTray', () => {
  it('无覆盖层且开启设置时，Escape 隐藏到托盘', () => {
    const wrapper = mountHarness()

    pressEscape()

    expect(invokeMock).toHaveBeenCalledWith('hide_to_tray')
    wrapper.unmount()
  })

  it('存在注册的活跃覆盖层时，Escape 不隐藏到托盘', () => {
    const wrapper = mountHarness()
    const unregister = registerOverlayActive(() => true)

    pressEscape()

    expect(invokeMock).not.toHaveBeenCalled()
    unregister()
    wrapper.unmount()
  })

  it('覆盖层注销（关闭）后，Escape 恢复隐藏到托盘', () => {
    const wrapper = mountHarness()
    const unregister = registerOverlayActive(() => true)
    pressEscape()
    expect(invokeMock).not.toHaveBeenCalled()

    unregister()
    pressEscape()

    expect(invokeMock).toHaveBeenCalledWith('hide_to_tray')
    wrapper.unmount()
  })

  it('覆盖层判定在捕获阶段先于菜单关闭执行，避免菜单关闭后同一次 Escape 触发隐藏', () => {
    const wrapper = mountHarness()
    let active = true
    registerOverlayActive(() => active)
    const menu = document.createElement('div')
    menu.className = 'context-menu'
    menu.tabIndex = -1
    menu.addEventListener('keydown', () => {
      // 模拟菜单自身的 Escape 处理器：关闭菜单 → 活跃状态置 false
      active = false
    })
    document.body.appendChild(menu)

    pressEscape(menu)

    expect(active).toBe(false)
    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})

// ── 逐层退出：第一次 Esc 退出输入框，第二次 Esc 退出应用 ──────────────
describe('useEscapeToTray · 逐层退出', () => {
  it('焦点在 textarea：第一按只失焦，不隐藏到托盘', () => {
    const wrapper = mountHarness()
    const ta = mountFocused(document.createElement('textarea'))
    expect(document.activeElement).toBe(ta)

    pressEscape(ta)

    expect(document.activeElement).toBe(document.body)
    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('textarea 失焦后再按：第二按隐藏到托盘', () => {
    const wrapper = mountHarness()
    const ta = mountFocused(document.createElement('textarea'))

    pressEscape(ta) // 第一按：退出输入框
    expect(invokeMock).not.toHaveBeenCalled()

    pressEscape(document) // 第二按：退出应用
    expect(invokeMock).toHaveBeenCalledWith('hide_to_tray')
    wrapper.unmount()
  })

  it('焦点在文本 input（含 search/number/password）：第一按只失焦', () => {
    const wrapper = mountHarness()
    for (const type of ['text', 'search', 'number', 'password']) {
      vi.clearAllMocks()
      const input = document.createElement('input')
      input.type = type
      mountFocused(input)

      pressEscape(input)

      expect(document.activeElement).not.toBe(input)
      expect(invokeMock).not.toHaveBeenCalled()
      input.remove()
    }
    wrapper.unmount()
  })

  it('未声明 type 的 input 视同文本输入', () => {
    const wrapper = mountHarness()
    const input = mountFocused(document.createElement('input'))

    pressEscape(input)

    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('checkbox / radio / button 不视为文本输入：Esc 直接隐藏到托盘', () => {
    const wrapper = mountHarness()
    for (const type of ['checkbox', 'radio', 'button', 'submit']) {
      vi.clearAllMocks()
      const input = document.createElement('input')
      input.type = type
      mountFocused(input)

      pressEscape(input)

      expect(invokeMock).toHaveBeenCalledWith('hide_to_tray')
      input.remove()
    }
    wrapper.unmount()
  })

  it('contenteditable：第一按只失焦', () => {
    const wrapper = mountHarness()
    const div = document.createElement('div')
    // 用特性而非属性：jsdom 实现了 contentEditable 反射但 contentEditable 的
    // isContentEditable 恒为 undefined，设特性更接近真实 DOM 形态
    div.setAttribute('contenteditable', 'true')
    mountFocused(div)

    pressEscape(div)

    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('data-esc-local 的输入框：全局跳过，不 blur 不退出（由控件自管）', () => {
    const wrapper = mountHarness()
    const input = document.createElement('input')
    input.type = 'text'
    input.setAttribute('data-esc-local', '')
    mountFocused(input)

    pressEscape(input)

    expect(document.activeElement).toBe(input) // 焦点保留
    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('IME 组合期：Esc 完全不介入（不 blur 不退出）', () => {
    const wrapper = mountHarness()
    const ta = mountFocused(document.createElement('textarea'))

    pressEscape(ta, { isComposing: true })

    expect(document.activeElement).toBe(ta) // 焦点保留，交给输入法
    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('覆盖层打开时，即使焦点在输入框也优先让覆盖层处理', () => {
    const wrapper = mountHarness()
    const unregister = registerOverlayActive(() => true)
    const ta = mountFocused(document.createElement('textarea'))

    pressEscape(ta)

    expect(document.activeElement).toBe(ta) // 不抢焦点
    expect(invokeMock).not.toHaveBeenCalled()
    unregister()
    wrapper.unmount()
  })

  it('未开启最小化到托盘：输入框失焦后仍不隐藏', () => {
    const wrapper = mountHarness({ enabled: false })
    const ta = mountFocused(document.createElement('textarea'))

    pressEscape(ta)
    pressEscape(document)

    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
