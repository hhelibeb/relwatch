import { describe, it, expect, vi } from 'vitest'
import { defineComponent } from 'vue'
import { mount } from '@vue/test-utils'
import { useOverlayEscape } from '../composables/useOverlayEscape'
import { hasActiveOverlay } from '../composables/contextMenuBus'

describe('useOverlayEscape', () => {
  it('挂载期间注册覆盖层，卸载后自动注销', () => {
    const onEscape = vi.fn()
    const Comp = defineComponent({
      setup() {
        useOverlayEscape({ onEscape })
        return {}
      },
      template: '<div />',
    })

    expect(hasActiveOverlay()).toBe(false)
    const wrapper = mount(Comp)
    expect(hasActiveOverlay()).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(onEscape).toHaveBeenCalledTimes(1)

    wrapper.unmount()
    expect(hasActiveOverlay()).toBe(false)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(onEscape).toHaveBeenCalledTimes(1)
  })

  it('忽略 IME 组合期和已 defaultPrevented 的事件，并透传其他按键', () => {
    const onEscape = vi.fn()
    const onKeydown = vi.fn()
    const Comp = defineComponent({
      setup() {
        useOverlayEscape({ onEscape, onKeydown })
        return {}
      },
      template: '<div />',
    })

    const wrapper = mount(Comp)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', isComposing: true }))
    const prevented = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    prevented.preventDefault()
    window.dispatchEvent(prevented)
    expect(onEscape).not.toHaveBeenCalled()

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'F3' }))
    expect(onKeydown).toHaveBeenCalledTimes(1)

    wrapper.unmount()
  })
})
