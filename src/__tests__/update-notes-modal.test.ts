import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { mount, enableAutoUnmount } from '@vue/test-utils'
import UpdateNotesModal from '../components/UpdateNotesModal.vue'
import { openReleaseUrl } from '../api/client'

vi.mock('../api/client', () => ({ openReleaseUrl: vi.fn() }))

const openReleaseUrlMock = vi.mocked(openReleaseUrl)

const NOTES = '## 新功能\n\n- 应用内显示 Release Note\n- [发布页](https://example.com)'

// 组件用 Teleport 挂到 body：DOM 不在 wrapper 内，一律经 document 查询
function mountModal(props: { version: string; date: string | null; body: string | null }) {
  return mount(UpdateNotesModal, { props, attachTo: document.body })
}

/** 取弹窗内的按钮（footer 的 .btn-sm，按 DOM 顺序） */
function footerButtons(): HTMLButtonElement[] {
  return Array.from(document.body.querySelectorAll('.update-notes-footer .btn-sm'))
}

// UpdateNotesModal 在 onMounted 里注册 window keydown 监听 + registerOverlayActive，
// 只在 onUnmounted 中清理；若某条用例断言失败提前抛错，末尾手工 wrapper.unmount()
// 就跳不到——document.body.innerHTML='' 只暴力摘 DOM，不会触发 Vue 卸载生命周期，
// 监听与 overlayStates 会跨用例泄漏。用 enableAutoUnmount 让每个用例结束自动卸载，
// 无论断言是否抛错都能收敛。
enableAutoUnmount(afterEach)

describe('UpdateNotesModal（应用内 Release Note 弹窗）', () => {
  beforeEach(() => {
    openReleaseUrlMock.mockClear()
  })

  afterEach(() => {
    // 兜底清 DOM（Teleport 内容即便组件卸载也可能残留）；卸载本身已由 enableAutoUnmount 处理
    document.body.innerHTML = ''
  })

  it('渲染版本号与 Markdown 正文', () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: NOTES })
    expect(document.body.textContent).toContain('v1.14.0')
    const html = document.body.querySelector('.markdown-body')!.innerHTML
    expect(html).toContain('<h2>')
    expect(html).toContain('应用内显示 Release Note')
    // 链接保留可点击（实际跳转由 App.vue 的全局外链守卫接管）
    expect(html).toContain('<a href="https://example.com"')
    wrapper.unmount()
  })

  it('date 为空时不渲染日期行（latest.json 可能不带 pub_date）', () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: NOTES })
    expect(document.body.querySelector('.update-notes-date')).toBeNull()
    wrapper.unmount()
  })

  it('date 有值时渲染本地化构建时间', () => {
    const wrapper = mountModal({ version: '1.14.0', date: '2026-08-30T10:00:00Z', body: NOTES })
    const dateEl = document.body.querySelector('.update-notes-date')
    expect(dateEl).toBeTruthy()
    // 不得残留字面占位符（t() 的 {0} 由格式化时间替换）
    expect(dateEl!.textContent).not.toContain('{0}')
    wrapper.unmount()
  })

  it('body 为空时显示空态而不是空白框', () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: null })
    expect(document.body.querySelector('.update-notes-empty')).toBeTruthy()
    expect(document.body.querySelector('.markdown-body')).toBeNull()
    wrapper.unmount()
  })

  it('「在浏览器中查看」打开该版本 Release 页', async () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: NOTES })
    footerButtons()[0].click()
    await Promise.resolve()
    expect(openReleaseUrlMock).toHaveBeenCalledWith(
      'https://github.com/hhelibeb/relwatch/releases/tag/v1.14.0',
    )
    wrapper.unmount()
  })

  it('「关闭」按钮与 Esc 均触发 close', async () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: NOTES })
    const buttons = footerButtons()
    buttons[buttons.length - 1].click()
    await Promise.resolve()
    expect(wrapper.emitted('close')).toHaveLength(1)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await Promise.resolve()
    expect(wrapper.emitted('close')).toHaveLength(2)
    wrapper.unmount()
  })

  it('点击遮罩空白处关闭', async () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: NOTES })
    const overlay = document.body.querySelector('.update-notes-overlay') as HTMLElement
    overlay.click()
    await Promise.resolve()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('卸载后不再响应 Esc（监听器已注销，防泄漏误触发）', () => {
    const wrapper = mountModal({ version: '1.14.0', date: null, body: NOTES })
    wrapper.unmount()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(wrapper.emitted('close')).toBeUndefined()
  })
})
