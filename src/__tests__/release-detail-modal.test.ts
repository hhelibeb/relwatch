import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import ReleaseDetailModal from '../components/ReleaseDetailModal.vue'
import { openReleaseUrl, copyImageToClipboard, copyTextToClipboard } from '../api/client'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  copyImageToClipboard: vi.fn(),
  copyTextToClipboard: vi.fn(),
}))

const openReleaseUrlMock = vi.mocked(openReleaseUrl)
const copyImageMock = vi.mocked(copyImageToClipboard)
const copyTextMock = vi.mocked(copyTextToClipboard)

function makeRelease(body: string | null): ReleaseInfo {
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'o',
    repo: 'r',
    tag_name: 'v1.0.0',
    release_name: 'v1.0.0',
    html_url: 'https://github.com/o/r/releases/tag/v1.0.0',
    published_at: '2024-01-01T00:00:00Z',
    prerelease: false,
    body,
    detected_at: '2024-01-01T00:00:00Z',
    notification_status: 'clicked',
    snooze_until: null,
    ai_summary: null,
    ai_importance: null,
    body_translated: null,
    extra_metadata: null,
    source_description: null,
  }
}

const BODY_WITH_LINK_AND_IMAGE =
  'see [release notes](https://example.com/notes) and ![shot](https://img.example.com/p.png) end'

function mountModal(body: string | null = BODY_WITH_LINK_AND_IMAGE) {
  return mount(ReleaseDetailModal, {
    props: { release: makeRelease(body), position: 1, total: 1, hasPrev: false, hasNext: false },
  })
}

function contextmenu(target: Element) {
  target.dispatchEvent(
    new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 100, clientY: 200 }),
  )
}

function menuButtons(): HTMLButtonElement[] {
  return Array.from(document.body.querySelectorAll('.context-menu button'))
}

function menuLabels(): string[] {
  return menuButtons().map(b => b.textContent?.trim() ?? '')
}

beforeEach(() => {
  copyTextMock.mockResolvedValue(undefined)
})

afterEach(() => {
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

describe('ReleaseDetailModal 正文右键菜单', () => {
  it('右键链接：显示「打开/复制链接」，复制链接写入剪贴板', async () => {
    const wrapper = mountModal()
    const anchor = document.body.querySelector('.release-detail-body a')!
    expect(anchor).toBeTruthy()

    contextmenu(anchor)
    await nextTick()

    expect(menuLabels()).toEqual(['打开', '复制链接'])

    menuButtons()[1].click()
    await nextTick()
    expect(copyTextMock).toHaveBeenCalledWith('https://example.com/notes')
    expect(document.body.querySelector('.context-menu')).toBeNull()
    wrapper.unmount()
  })

  it('右键链接选择「打开」：调用系统浏览器而非应用内导航', async () => {
    const wrapper = mountModal()
    const anchor = document.body.querySelector('.release-detail-body a')!

    contextmenu(anchor)
    await nextTick()
    menuButtons()[0].click()
    await nextTick()

    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://example.com/notes')
    wrapper.unmount()
  })

  it('右键图片：显示「复制图片/复制图片链接/打开」，复制图片走下载+转码流程', async () => {
    const wrapper = mountModal()
    const img = document.body.querySelector('.release-detail-body img')!
    expect(img).toBeTruthy()

    contextmenu(img)
    await nextTick()

    expect(menuLabels()).toEqual(['复制图片', '复制图片链接', '打开'])

    copyImageMock.mockResolvedValue(undefined)
    menuButtons()[0].click()
    await nextTick()
    expect(copyImageMock).toHaveBeenCalledWith('https://img.example.com/p.png')
    wrapper.unmount()
  })

  it('右键图片「打开」：在浏览器打开图片地址', async () => {
    const wrapper = mountModal()
    const img = document.body.querySelector('.release-detail-body img')!

    contextmenu(img)
    await nextTick()
    menuButtons()[2].click()
    await nextTick()

    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://img.example.com/p.png')
    wrapper.unmount()
  })

  it('右键普通文本（无选区）：显示「复制内容」并复制整篇正文', async () => {
    const wrapper = mountModal()
    const body = document.body.querySelector('.release-detail-body')!

    contextmenu(body)
    await nextTick()

    expect(menuLabels()).toEqual(['复制内容'])

    menuButtons()[0].click()
    await nextTick()
    expect(copyTextMock).toHaveBeenCalledWith(BODY_WITH_LINK_AND_IMAGE)
    wrapper.unmount()
  })

  it('菜单打开时 Esc 只关菜单不关弹窗，再次 Esc 才关闭弹窗', async () => {
    const wrapper = mountModal()
    const anchor = document.body.querySelector('.release-detail-body a')!

    contextmenu(anchor)
    await nextTick()
    expect(document.body.querySelector('.context-menu')).not.toBeNull()

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    await nextTick()
    expect(document.body.querySelector('.context-menu')).toBeNull()
    expect(wrapper.emitted('close')).toBeUndefined()

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    await nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })
})

describe('ReleaseDetailModal 内容视图切换（摘要 / 译文 / 原文）', () => {
  function makeFullRelease(): ReleaseInfo {
    return { ...makeRelease('## 原文内容'), ai_summary: '这是摘要', body_translated: '## 译文内容' }
  }

  function mountFull(release: ReleaseInfo) {
    return mount(ReleaseDetailModal, {
      props: { release, position: 1, total: 1, hasPrev: false, hasNext: false },
    })
  }

  function tabLabels(): string[] {
    return Array.from(document.body.querySelectorAll('.release-detail-tabs .release-view-tab'))
      .map(b => b.textContent?.trim() ?? '')
  }

  it('三种内容齐备时显示摘要/译文/原文标签，默认选中译文并渲染译文', () => {
    const wrapper = mountFull(makeFullRelease())

    expect(tabLabels()).toEqual(['摘要', '译文', '原文'])
    expect(document.body.querySelector('.release-view-tab.active')?.textContent?.trim()).toBe('译文')
    expect(document.body.querySelector('.release-detail-markdown')?.textContent).toContain('译文内容')
    wrapper.unmount()
  })

  it('点击摘要标签切换显示摘要内容', async () => {
    const wrapper = mountFull(makeFullRelease())

    const tabs = Array.from(document.body.querySelectorAll<HTMLButtonElement>('.release-detail-tabs .release-view-tab'))
    tabs[0].click()
    await nextTick()

    expect(document.body.querySelector('.release-view-tab.active')?.textContent?.trim()).toBe('摘要')
    expect(document.body.querySelector('.release-detail-markdown')?.textContent).toContain('这是摘要')
    wrapper.unmount()
  })

  it('无译文时默认选中原版', () => {
    const wrapper = mountFull({ ...makeRelease('## 原文内容'), ai_summary: '这是摘要' })

    expect(tabLabels()).toEqual(['摘要', '原文'])
    expect(document.body.querySelector('.release-view-tab.active')?.textContent?.trim()).toBe('原文')
    wrapper.unmount()
  })
})
