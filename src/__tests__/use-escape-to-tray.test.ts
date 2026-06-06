import { describe, it, expect, vi, afterEach } from 'vitest'
import { defineComponent, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { useEscapeToTray } from '../composables/useEscapeToTray'

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

function pressEscape(target: EventTarget = document) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
}

afterEach(() => {
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

describe('useEscapeToTray', () => {
  it('无覆盖层且开启设置时，Escape 隐藏到托盘', () => {
    const wrapper = mount(Harness)

    pressEscape()

    expect(invokeMock).toHaveBeenCalledWith('hide_to_tray')
    wrapper.unmount()
  })

  it('存在右键菜单时，Escape 不隐藏到托盘', () => {
    const wrapper = mount(Harness)
    document.body.innerHTML = '<div class="context-menu"><button>copy</button></div>'

    pressEscape()

    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('全局监听在捕获阶段先于菜单关闭执行，避免菜单关闭后同一次 Escape 触发隐藏', () => {
    const wrapper = mount(Harness)
    const menu = document.createElement('div')
    menu.className = 'context-menu'
    menu.tabIndex = -1
    menu.addEventListener('keydown', () => {
      menu.remove()
    })
    document.body.appendChild(menu)

    pressEscape(menu)

    expect(document.querySelector('.context-menu')).toBeNull()
    expect(invokeMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
