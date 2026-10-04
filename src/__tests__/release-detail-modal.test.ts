import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { nextTick, reactive, ref } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'
import ReleaseDetailModal from '../components/ReleaseDetailModal.vue'
import MarkdownContent from '../components/common/MarkdownContent.vue'
import { ShowImportanceKey, AiEnabledKey } from '../injection-keys'
import { openReleaseUrl, copyImageToClipboard, copyTextToClipboard } from '../api/client'
import { translateRelease } from '../api/releases'
import { events } from '../bindings'
import type { ReleaseTranslateChunk } from '../bindings'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  copyImageToClipboard: vi.fn(),
  copyTextToClipboard: vi.fn(),
}))

vi.mock('../api/releases', () => ({
  translateRelease: vi.fn(),
  setReleaseFlag: vi.fn(),
}))

// 分片由后端经事件推进：测试持住监听回调自行投递（真 Tauri 环境外无处可发）
vi.mock('../bindings', () => ({
  events: {
    releaseTranslateChunk: { listen: vi.fn() },
  },
}))

const translateReleaseMock = vi.mocked(translateRelease)

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
    flag: 0,
    version_bump: null,
  }
}

const BODY_WITH_LINK_AND_IMAGE =
  'see [release notes](https://example.com/notes) and ![shot](https://img.example.com/p.png) end'

function mountModal(body: string | null = BODY_WITH_LINK_AND_IMAGE) {
  return mount(ReleaseDetailModal, {
    props: { release: makeRelease(body), position: 1, total: 1, hasPrev: false, hasNext: false },
  })
}

function mountModalWithRelease(release: ReleaseInfo, provide: Record<symbol, unknown> = {}) {
  return mount(ReleaseDetailModal, {
    props: { release, position: 1, total: 1, hasPrev: false, hasNext: false },
    global: { provide },
  })
}

// 详情弹窗与卡片共用同一套长文本策略（仓库名/版本号可省略 + title 兜底）
describe('ReleaseDetailModal — 长文本截断兜底', () => {
  it('仓库名与版本号都带 title（截断后可取回完整值）', async () => {
    const release = {
      ...makeRelease(null),
      owner: 'example-org',
      repo: 'example-repo',
      tag_name: 'dsh-v0.1.7-rc.2',
    }
    const wrapper = mountModalWithRelease(release)
    await nextTick()

    expect(document.body.querySelector('.release-detail-repo')?.getAttribute('title'))
      .toBe('example-org/example-repo')
    expect(document.body.querySelector('.release-detail-tag')?.getAttribute('title'))
      .toBe('dsh-v0.1.7-rc.2')
    wrapper.unmount()
  })
})

describe('ReleaseDetailModal — 显示重要度开关', () => {
  it('默认显示 AI 重要度徽标，开关关闭后隐藏', async () => {
    const release = { ...makeRelease(null), ai_importance: '大' }

    // modal 经 Teleport 渲染到 body，断言走 document.body
    const on = mountModalWithRelease(release, { [ShowImportanceKey as symbol]: ref(true) })
    await nextTick()
    expect(document.body.querySelectorAll('.release-importance-chip').length).toBe(1)
    on.unmount()
    expect(document.body.querySelector('.release-importance-chip')).toBeNull()

    const off = mountModalWithRelease(release, { [ShowImportanceKey as symbol]: ref(false) })
    await nextTick()
    expect(document.body.querySelector('.release-importance-chip')).toBeNull()
  })
})

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

describe('ReleaseDetailModal — 流式翻译', () => {
  type ChunkEvent = { payload: ReleaseTranslateChunk }
  let chunkHandler: ((e: ChunkEvent) => void) | null = null

  function emitChunk(releaseId: number, delta: string) {
    chunkHandler?.({ payload: { release_id: releaseId, delta } })
  }

  /** 等过合帧窗口（50ms，见 useReleaseTranslate 的 STREAM_FLUSH_MS）。 */
  function waitFrame() {
    return new Promise((r) => setTimeout(r, 60))
  }

  /** 可控完成时机的命令承诺：先投分片再放行，否则命令会在分片到达前就收场。 */
  function deferred() {
    let resolve!: () => void
    let reject!: (e: unknown) => void
    const promise = new Promise<void>((res, rej) => {
      resolve = res
      reject = rej
    })
    return { promise, resolve, reject }
  }

  function mountWithAi(release: ReleaseInfo) {
    return mountModalWithRelease(release, { [AiEnabledKey as symbol]: ref(true) })
  }

  function actionButton(label: string): HTMLButtonElement | undefined {
    return Array.from(document.body.querySelectorAll<HTMLButtonElement>('.release-detail-actions .btn-sm'))
      .find((b) => b.textContent?.trim() === label)
  }

  function bodyText(): string {
    return document.body.querySelector('.release-detail-markdown')?.textContent ?? ''
  }

  function interruptedNotice(): Element | null {
    return document.body.querySelector('.release-detail-interrupted')
  }

  beforeEach(() => {
    chunkHandler = null
    vi.mocked(events.releaseTranslateChunk.listen).mockImplementation((cb) => {
      chunkHandler = cb as unknown as (e: ChunkEvent) => void
      return Promise.resolve(vi.fn())
    })
  })

  it('分片边收边渲染，不再只显示「正在翻译」提示', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const wrapper = mountWithAi(makeRelease('## 原文内容'))

    actionButton(t('context.translate'))!.click()
    await flushPromises()

    // 分片未到时仍是提示
    expect(document.body.querySelector('.release-detail-translating')?.textContent).toContain(t('release.translating_hint'))

    emitChunk(1, '## 译文')
    await waitFrame()

    expect(bodyText()).toContain('译文')
    expect(document.body.querySelector('.release-detail-translating')).toBeNull()

    // 流式缓冲不写 Markdown 渲染缓存（内容每帧都变，会冲掉静态场景的缓存条目）
    expect((wrapper.findComponent(MarkdownContent).props() as { noCache?: boolean }).noCache).toBe(true)
    // 还在出字：没有中断可言
    expect(interruptedNotice()).toBeNull()

    cmd.resolve()
    await flushPromises()
    wrapper.unmount()
  })

  it('流式缓冲在缓冲期渲染，落库后交回 props（不重复显示）', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease('## 原文内容'))
    const wrapper = mountWithAi(release)

    actionButton(t('context.translate'))!.click()
    await flushPromises()

    emitChunk(1, '半截译文')
    await waitFrame()
    expect(bodyText()).toContain('半截译文')

    cmd.resolve()
    await flushPromises()
    // 列表刷新前仍靠缓冲兜住（否则这里会闪一下空白）
    expect(bodyText()).toContain('半截译文')

    release.body_translated = '完整译文'
    await nextTick()

    expect(bodyText()).toContain('完整译文')
    expect(bodyText()).not.toContain('半截译文')
    expect((wrapper.findComponent(MarkdownContent).props() as { noCache?: boolean }).noCache).toBe(false)
    // 落库了就是完整译文，提示必须消失
    expect(interruptedNotice()).toBeNull()
    wrapper.unmount()
  })

  it('翻译失败但有分片：保留部分译文、停在译文视图，且仍可逆重试', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease('## 原文内容'))
    const wrapper = mountWithAi(release)

    actionButton(t('context.translate'))!.click()
    await flushPromises()
    emitChunk(1, '半截译文')
    await waitFrame()

    cmd.reject(new Error('boom'))
    await flushPromises()

    // 不回退到原文：部分译文是此刻唯一能看到的内容
    expect(bodyText()).toContain('半截译文')
    expect(bodyText()).not.toContain('原文内容')
    // 末尾红字中断提示：不完整必须说出来，否则会被当完整译文引用/复制
    expect(interruptedNotice()?.textContent).toContain(t('release.translate_interrupted'))
    // 标签不再是「翻译中...」（已收场）
    const activeTab = document.body.querySelector('.release-view-tab.active')?.textContent?.trim()
    expect(activeTab).toBe(t('release.view_translated'))

    // 重试入口留着：按钮消失等于逼用户先切回原文
    const retry = actionButton(t('context.translate'))
    expect(retry).toBeTruthy()
    expect(retry!.disabled).toBe(false)

    retry!.click()
    await flushPromises()
    expect(translateReleaseMock).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })

  it('翻译失败且无任何分片：回退原文（既有行为）', async () => {
    translateReleaseMock.mockRejectedValue(new Error('boom'))
    const wrapper = mountWithAi(makeRelease('## 原文内容'))

    actionButton(t('context.translate'))!.click()
    await flushPromises()

    expect(bodyText()).toContain('原文内容')
    expect(document.body.querySelector('.release-detail-translating')).toBeNull()
    // 失败后译文标签消失（无译文可看）
    const labels = Array.from(document.body.querySelectorAll('.release-view-tab')).map((b) => b.textContent?.trim())
    expect(labels).not.toContain(t('release.view_translated'))
    // 什么都没有生成：没有可标记为「不完整」的内容
    expect(interruptedNotice()).toBeNull()
    wrapper.unmount()
  })

  it('跟随到底部只在收字时生效：切版本清空缓冲不会把视图又拽回底部', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease('## 原文内容'))
    const wrapper = mountWithAi(release)

    actionButton(t('context.translate'))!.click()
    await flushPromises()

    // jsdom 无布局：尺寸与滚动全靠打的桩（scrollTop 900 即「贴底」）
    const body = document.body.querySelector('.release-detail-body') as HTMLElement
    const scrolled: number[] = []
    Object.defineProperty(body, 'scrollHeight', { value: 1000, configurable: true })
    Object.defineProperty(body, 'clientHeight', { value: 100, configurable: true })
    Object.defineProperty(body, 'scrollTop', {
      get: () => 900,
      set: (v: number) => { scrolled.push(v) },
      configurable: true,
    })
    Object.defineProperty(body, 'scrollTo', { value: () => {}, configurable: true })

    emitChunk(1, '半截译文')
    await waitFrame()
    await nextTick()
    expect(scrolled).toContain(1000)

    // 切版本：清空缓冲同样触发跟随的 watcher，但此刻显示的已不是这条缓冲
    scrolled.length = 0
    release.id = 8
    await nextTick()
    await nextTick()
    await flushPromises()
    expect(scrolled).toEqual([])

    wrapper.unmount()
  })
})
