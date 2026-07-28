import { describe, it, expect, afterEach, vi } from 'vitest'
import { defineComponent } from 'vue'
import { mount } from '@vue/test-utils'
import { useExternalLinkGuard } from '../composables/useExternalLinkGuard'
import { openReleaseUrl } from '../api/client'

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))

const openReleaseUrlMock = vi.mocked(openReleaseUrl)

const Harness = defineComponent({
  setup() {
    useExternalLinkGuard()
    return {}
  },
  template: `
    <div>
      <a class="external" href="https://x.com/post/1" target="_blank">external</a>
      <a class="relative" href="/some/relative/path">relative</a>
      <button class="plain">button</button>
    </div>
  `,
})

afterEach(() => {
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

describe('useExternalLinkGuard', () => {
  it('点击 http(s) 链接：阻止默认导航并交给系统浏览器打开', () => {
    const wrapper = mount(Harness, { attachTo: document.body })
    const link = wrapper.find('a.external').element

    const event = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })
    link.dispatchEvent(event)

    expect(event.defaultPrevented).toBe(true)
    expect(openReleaseUrlMock).toHaveBeenCalledTimes(1)
    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://x.com/post/1')
    wrapper.unmount()
  })

  it('点击相对路径链接：吞掉（不导航 webview，也不打开浏览器）', () => {
    const wrapper = mount(Harness, { attachTo: document.body })
    const link = wrapper.find('a.relative').element

    const event = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })
    link.dispatchEvent(event)

    expect(event.defaultPrevented).toBe(true)
    expect(openReleaseUrlMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('点击非链接元素：不拦截', () => {
    const wrapper = mount(Harness, { attachTo: document.body })

    wrapper.find('button.plain').element.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 }),
    )

    expect(openReleaseUrlMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('链接内的子元素点击同样被拦截（closest 向上查找）', () => {
    const wrapper = mount(Harness, { attachTo: document.body })
    const link = wrapper.find('a.external').element as HTMLAnchorElement
    const span = document.createElement('span')
    span.textContent = 'inner'
    link.appendChild(span)

    const event = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })
    span.dispatchEvent(event)

    expect(event.defaultPrevented).toBe(true)
    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://x.com/post/1')
    wrapper.unmount()
  })

  it('拦截后阻止传播：元素自身的 click 监听不再触发', () => {
    const wrapper = mount(Harness, { attachTo: document.body })
    const link = wrapper.find('a.external').element
    const onClick = vi.fn()
    link.addEventListener('click', onClick)

    link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 }))

    expect(onClick).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
