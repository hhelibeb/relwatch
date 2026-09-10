import { describe, expect, it, vi, afterEach } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { useContextMenu } from '../composables/useContextMenu'
import { openReleaseUrl, copyTextToClipboard } from '../api/client'
import { ShowToastKey } from '../injection-keys'

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  copyTextToClipboard: vi.fn(),
}))

const openReleaseUrlMock = vi.mocked(openReleaseUrl)
const copyTextToClipboardMock = vi.mocked(copyTextToClipboard)

const ContextMenuHarness = defineComponent({
  setup() {
    const {
      contextMenu,
      handleContextMenu,
      handleCopyLink,
      handleOpenLink,
    } = useContextMenu()
    return { contextMenu, handleContextMenu, handleCopyLink, handleOpenLink }
  },
  template: `
    <div>
      <button class="target" @contextmenu.prevent="handleContextMenu($event, 'https://example.com/release')">open menu</button>
      <div v-if="contextMenu" class="menu" :data-x="contextMenu.x" :data-y="contextMenu.y" :data-url="contextMenu.url">
        <button class="copy" @click="handleCopyLink">copy</button>
        <button class="open" @click="handleOpenLink">open</button>
      </div>
    </div>
  `,
})

afterEach(() => {
  vi.clearAllMocks()
})

describe('useContextMenu — 通用关闭行为', () => {
  it('右键打开菜单并在 document click 后关闭', async () => {
    const wrapper = mount(ContextMenuHarness)

    await wrapper.get('.target').trigger('contextmenu', { clientX: 12, clientY: 34 })
    const menu = wrapper.get('.menu')
    expect(menu.attributes('data-url')).toBe('https://example.com/release')
    expect(menu.attributes('data-x')).toBe('12')
    expect(menu.attributes('data-y')).toBe('34')

    document.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()

    expect(wrapper.find('.menu').exists()).toBe(false)
    wrapper.unmount()
  })

  it('打开新菜单时会通过 contextMenuBus 关闭已打开菜单', async () => {
    const first = mount(ContextMenuHarness)
    const second = mount(ContextMenuHarness)

    await first.get('.target').trigger('contextmenu', { clientX: 1, clientY: 2 })
    expect(first.find('.menu').exists()).toBe(true)

    await second.get('.target').trigger('contextmenu', { clientX: 3, clientY: 4 })

    expect(first.find('.menu').exists()).toBe(false)
    expect(second.find('.menu').exists()).toBe(true)

    first.unmount()
    second.unmount()
  })

  it('打开链接动作会调用 openReleaseUrl 并关闭菜单', async () => {
    const wrapper = mount(ContextMenuHarness)

    await wrapper.get('.target').trigger('contextmenu')
    await wrapper.get('.open').trigger('click')

    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://example.com/release')
    expect(wrapper.find('.menu').exists()).toBe(false)
    wrapper.unmount()
  })

  it('复制链接动作统一走 copyTextToClipboard，成功后关闭菜单', async () => {
    copyTextToClipboardMock.mockResolvedValue(undefined)
    const wrapper = mount(ContextMenuHarness)

    await wrapper.get('.target').trigger('contextmenu')
    await wrapper.get('.copy').trigger('click')
    await nextTick()

    expect(copyTextToClipboardMock).toHaveBeenCalledWith('https://example.com/release')
    expect(wrapper.find('.menu').exists()).toBe(false)
    wrapper.unmount()
  })

  it('复制链接失败时通过 Toast 给出反馈（不再静默失败）', async () => {
    copyTextToClipboardMock.mockRejectedValueOnce(new Error('boom'))
    const showToast = vi.fn()
    const wrapper = mount(ContextMenuHarness, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })

    await wrapper.get('.target').trigger('contextmenu')
    await wrapper.get('.copy').trigger('click')
    await nextTick()
    await nextTick()

    expect(showToast).toHaveBeenCalledTimes(1)
    expect(String(showToast.mock.calls[0][0])).toContain('boom')
    wrapper.unmount()
  })
})
