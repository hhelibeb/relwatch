import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount, enableAutoUnmount } from '@vue/test-utils'
import { nextTick } from 'vue'
import BaseModal from '../components/ui/BaseModal.vue'
import BaseTooltip from '../components/ui/BaseTooltip.vue'
import BaseSelect from '../components/ui/BaseSelect.vue'
import BaseBulkBar from '../components/ui/BaseBulkBar.vue'
import AppIcon from '../components/ui/AppIcon.vue'
import { invalidateTransientUi, endTransientUiInvalidation } from '../composables/contextMenuBus'

enableAutoUnmount(afterEach)

afterEach(() => {
  endTransientUiInvalidation()
  document.body.innerHTML = ''
})

describe('BaseModal', () => {
  it('Teleport 到 body，支持 Esc、遮罩点击与关闭时恢复焦点', async () => {
    const trigger = document.createElement('button')
    document.body.appendChild(trigger)
    trigger.focus()
    expect(document.activeElement).toBe(trigger)

    const wrapper = mount(BaseModal, {
      props: { ariaLabel: 'Test Dialog' },
      slots: { default: '<button class="first-btn">First</button><button class="last-btn">Last</button>' },
    })
    await nextTick()

    const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement
    expect(dialog).not.toBeNull()
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    expect(dialog.getAttribute('aria-label')).toBe('Test Dialog')

    const first = dialog.querySelector('.first-btn') as HTMLButtonElement
    const last = dialog.querySelector('.last-btn') as HTMLButtonElement
    expect(document.activeElement).toBe(first)

    // Shift+Tab 从首个可聚焦元素循环到末尾
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true }))
    expect(document.activeElement).toBe(last)

    // Tab 从末尾循环回首个
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }))
    expect(document.activeElement).toBe(first)

    // Esc 触发 close
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(wrapper.emitted('close')).toHaveLength(1)

    // 遮罩点击触发 close
    const overlay = document.body.querySelector('.base-modal-overlay') as HTMLElement
    overlay.click()
    expect(wrapper.emitted('close')).toHaveLength(2)

    wrapper.unmount()
    expect(document.activeElement).toBe(trigger)
  })

  it('closeOnBackdrop=false 阻止点击遮罩关闭，自定义 onEscape 优先执行，且 trapFocus=false 不拦截 Tab', async () => {
    const onEscape = vi.fn()
    const external = document.createElement('input')
    document.body.appendChild(external)

    const wrapper = mount(BaseModal, {
      props: { closeOnBackdrop: false, trapFocus: false, onEscape },
      slots: { default: '<button class="inside">Inside</button>' },
    })
    await nextTick()

    ;(document.body.querySelector('.base-modal-overlay') as HTMLElement).click()
    expect(wrapper.emitted('close')).toBeUndefined()

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(onEscape).toHaveBeenCalledTimes(1)
    expect(wrapper.emitted('close')).toBeUndefined()

    // 外部元素拿到焦点后，trapFocus=false 不拉回焦点，且卸载时不抢回旧焦点
    external.focus()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }))
    expect(document.activeElement).toBe(external)

    wrapper.unmount()
    expect(document.activeElement).toBe(external)
  })
})

describe('BaseTooltip', () => {
  it('按 visible 渲染到 body，并在瞬态失效期抑制显示', async () => {
    const wrapper = mount(BaseTooltip, {
      props: { visible: true, text: 'Tooltip body', x: 100, y: 120, tooltipClass: 'custom-tip' },
    })
    await nextTick()

    const tip = document.body.querySelector('.base-tooltip.custom-tip') as HTMLElement
    expect(tip).not.toBeNull()
    expect(tip.textContent).toBe('Tooltip body')
    expect(tip.getAttribute('role')).toBe('tooltip')

    invalidateTransientUi()
    await nextTick()
    expect(document.body.querySelector('.base-tooltip')).toBeNull()

    endTransientUiInvalidation()
    await nextTick()
    expect(document.body.querySelector('.base-tooltip')).not.toBeNull()

    // env.d.ts 的 *.vue shim 让纯 tsc 看不到组件 props（全可选 → 弱类型拒绝多余属性），vue-tsc 不受影响
    await wrapper.setProps({ visible: false } as never)
    expect(document.body.querySelector('.base-tooltip')).toBeNull()
  })

  it('锚点靠近底部时自动向上翻转并保持在视口内', async () => {
    const anchor = document.createElement('div')
    anchor.getBoundingClientRect = vi.fn().mockReturnValue({
      left: window.innerWidth - 20,
      top: window.innerHeight - 30,
      bottom: window.innerHeight - 5,
    })

    mount(BaseTooltip, {
      props: { visible: true, text: 'Anchored', anchorEl: anchor, elevated: true },
    })
    await nextTick()

    const tip = document.body.querySelector('.base-tooltip') as HTMLElement
    expect(tip).not.toBeNull()
    expect(tip.style.zIndex).toBe('var(--z-popover-modal)')
    expect(parseFloat(tip.style.top)).toBeLessThan(window.innerHeight - 30)
  })
})

describe('BaseSelect', () => {
  const options = [
    { value: 'light', label: '浅色' },
    { value: 'dark', label: '深色' },
  ]

  it('支持展开、悬停预览、离开恢复与选择回写', async () => {
    const wrapper = mount(BaseSelect, {
      props: { modelValue: 'light', options, label: '主题', previewOnHover: true },
    })

    await wrapper.get('.theme-select-trigger').trigger('click')
    const opts = wrapper.findAll('.theme-select-option')
    expect(opts).toHaveLength(2)

    await opts[1].trigger('mouseenter')
    expect(wrapper.emitted('preview')).toEqual([['dark']])

    await wrapper.get('.base-select').trigger('mouseleave')
    expect(wrapper.emitted('restore')).toHaveLength(1)

    await opts[1].trigger('click')
    expect(wrapper.emitted('update:modelValue')).toEqual([['dark']])
    expect(wrapper.emitted('select')).toEqual([['dark']])
    expect(wrapper.find('.theme-select-dropdown').exists()).toBe(false)
  })

  it('disabled 时点击不展开', async () => {
    const wrapper = mount(BaseSelect, {
      props: { modelValue: 'light', options, disabled: true },
    })
    await wrapper.get('.theme-select-trigger').trigger('click')
    expect(wrapper.find('.theme-select-dropdown').exists()).toBe(false)
  })

  it('非预览模式（previewOnHover=false）键盘导航不篡改触发器文案，且鼠标点击后焦点回到触发器', async () => {
    const wrapper = mount(BaseSelect, {
      props: { modelValue: 'light', options, previewOnHover: false },
      attachTo: document.body,
    })

    const trigger = wrapper.get('.theme-select-trigger')
    await trigger.trigger('click')
    await nextTick()

    const listbox = wrapper.get('.theme-select-dropdown')
    await listbox.trigger('keydown', { key: 'ArrowDown' })
    expect(trigger.text()).toBe('浅色')
    expect(wrapper.emitted('preview')).toBeUndefined()
    expect(wrapper.findAll('.theme-select-option')[0].classes()).toContain('selected')

    await wrapper.findAll('.theme-select-option')[1].trigger('click')
    await nextTick()
    expect(wrapper.emitted('update:modelValue')).toEqual([['dark']])
    expect(document.activeElement).toBe(trigger.element)
  })
})

describe('BaseBulkBar', () => {
  it('展示计数、禁用态并发射 selectAll / clear 事件', async () => {
    const wrapper = mount(BaseBulkBar, {
      props: { selectedCount: 0, selectedLabel: '已选 0 项' },
      slots: { default: '<button class="extra-action">删除</button>' },
    })

    expect(wrapper.get('.bulk-count').text()).toBe('已选 0 项')
    const btns = wrapper.findAll('button')
    expect(btns[1].attributes('disabled')).toBeDefined()
    expect(wrapper.find('.extra-action').exists()).toBe(true)

    await btns[0].trigger('click')
    expect(wrapper.emitted('selectAll')).toHaveLength(1)

    await wrapper.setProps({ selectedCount: 2, selectedLabel: '已选 2 项' } as never)
    await wrapper.findAll('button')[1].trigger('click')
    expect(wrapper.emitted('clear')).toHaveLength(1)
  })
})

describe('AppIcon', () => {
  it('渲染 sprite 引用并正确设置尺寸与无障碍属性', () => {
    const sized = mount(AppIcon, { props: { name: 'close-icon', size: 16 } })
    expect(sized.get('use').attributes('href')).toBe('/icons.svg#close-icon')
    expect(sized.get('svg').attributes('aria-hidden')).toBe('true')
    expect(sized.get('svg').attributes('style')).toContain('width: 16px')
    expect(sized.get('svg').attributes('style')).toContain('height: 16px')

    const plain = mount(AppIcon, { props: { name: 'close-icon' } })
    expect(plain.get('svg').attributes('style')).toBeUndefined()
  })
})
