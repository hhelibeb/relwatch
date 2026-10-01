import { describe, expect, it, vi, afterEach, beforeEach } from 'vitest'
import { defineComponent, nextTick, reactive } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'
import { useReleaseTranslate } from '../composables/useReleaseTranslate'
import { translateRelease } from '../api/releases'
import { events } from '../bindings'
import type { ReleaseTranslateChunk } from '../bindings'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'

vi.mock('../api/releases', () => ({
  translateRelease: vi.fn(),
}))

// 事件层单独 mock：分片由后端经 release-translate-chunk 事件推进，
// 测试需要拿到监听回调自行投递分片（真 Tauri 环境外无处可发）
vi.mock('../bindings', () => ({
  events: {
    releaseTranslateChunk: { listen: vi.fn() },
  },
}))

// 用量埋点与断言无关：track mock 为 no-op，避免真实实现的定时器与写入干扰
vi.mock('../composables/useUsageTracking', () => ({
  track: vi.fn(),
}))

const translateReleaseMock = vi.mocked(translateRelease)

function makeRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  return {
    id: 'rel-1',
    source_type: 'github',
    source_owner: 'o',
    source_repo: 'r',
    version: 'v1.0.0',
    title: 'v1.0.0',
    published_at: '2024-01-01T00:00:00Z',
    url: 'https://example.com/releases/v1',
    body: 'body',
    body_translated: null,
    importance: '中',
    unread: false,
    ...overrides,
  } as ReleaseInfo
}

function mountHarness(opts: {
  release: () => ReleaseInfo
  showToast?: (msg: string) => void
  onStart?: () => void
  onSuccess?: () => void
  onError?: () => void
  onTranslated?: () => void
  stream?: boolean
}) {
  return mount(defineComponent({
    setup() {
      const { translating, streamText, handleTranslateRelease } = useReleaseTranslate(opts)
      return { translating, streamText, handleTranslateRelease }
    },
    template: `
      <div>
        <button class="translate" @click="handleTranslateRelease">translate</button>
        <span class="busy" v-if="translating">busy</span>
        <span class="stream">{{ streamText }}</span>
      </div>
    `,
  }))
}

// ── 流式分片（详情弹窗专用）──
type ChunkEvent = { payload: ReleaseTranslateChunk }
let chunkHandler: ((e: ChunkEvent) => void) | null = null

/** 投递一个分片（模拟后端事件）。 */
function emitChunk(releaseId: number, delta: string) {
  chunkHandler?.({ payload: { release_id: releaseId, delta } })
}

/** 等过合帧窗口（50ms）——分片不是逐条上屏，定窗合并后才写入缓冲。 */
function waitFrame() {
  return new Promise((r) => setTimeout(r, 60))
}

/** 可控完成时机的命令承诺：先投分片、再放行成功/失败，
 *  否则命令会在分片到达前就结束（失败路径下缓冲会被立刻退订）。 */
function deferred() {
  let resolve!: () => void
  let reject!: (e: unknown) => void
  const promise = new Promise<void>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

beforeEach(() => {
  chunkHandler = null
  vi.mocked(events.releaseTranslateChunk.listen).mockImplementation((cb) => {
    chunkHandler = cb as unknown as (e: ChunkEvent) => void
    return Promise.resolve(vi.fn())
  })
})

afterEach(() => {
  vi.clearAllMocks()
})

describe('useReleaseTranslate — 翻译状态机', () => {
  it('翻译成功：调用命令、触发 onStart/onSuccess；translating 保持到 body_translated 落库后复位', async () => {
    translateReleaseMock.mockResolvedValue(undefined)
    // reactive 包装：与真实场景（列表数据响应式）一致，watch 依赖才可触发
    const release = reactive(makeRelease())
    const onStart = vi.fn()
    const onSuccess = vi.fn()
    const onTranslated = vi.fn()
    const wrapper = mountHarness({
      release: () => release,
      onStart,
      onSuccess,
      onTranslated,
    })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    expect(translateReleaseMock).toHaveBeenCalledWith('rel-1')
    expect(onStart).toHaveBeenCalledOnce()
    expect(onSuccess).toHaveBeenCalledOnce()
    // 命令成功后仍等待后端落库（body_translated 从无到有）才复位
    expect(wrapper.find('.busy').exists()).toBe(true)

    // 模拟列表刷新后 body_translated 生效
    release.body_translated = '译文'
    await nextTick()

    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(onTranslated).toHaveBeenCalledOnce()
  })

  it('翻译失败：translating 复位、toast 提示、触发 onError', async () => {
    translateReleaseMock.mockRejectedValue(new Error('boom'))
    const showToast = vi.fn()
    const onError = vi.fn()
    const wrapper = mountHarness({
      release: () => makeRelease(),
      showToast,
      onError,
    })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    expect(translateReleaseMock).toHaveBeenCalledWith('rel-1')
    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(showToast).toHaveBeenCalledWith(t('release.translate_failed') + 'boom')
    expect(onError).toHaveBeenCalledOnce()
  })

  it('翻译中保持 busy 状态（异步未完成前不复位）', async () => {
    let resolve!: (v: void) => void
    translateReleaseMock.mockImplementation(() => new Promise<void>(r => { resolve = r }))
    const wrapper = mountHarness({ release: () => makeRelease() })

    await wrapper.get('.translate').trigger('click')
    await nextTick()
    expect(wrapper.find('.busy').exists()).toBe(true)

    // 命令完成后仍 busy（body_translated 未落库）
    resolve()
    await flushPromises()
    expect(wrapper.find('.busy').exists()).toBe(true)
  })

  it('body_translated 从无到有（成功落库后响应式生效）复位 translating 并触发 onTranslated', async () => {
    translateReleaseMock.mockResolvedValue(undefined)
    const release = reactive(makeRelease())
    const onTranslated = vi.fn()
    const wrapper = mountHarness({
      release: () => release,
      onTranslated,
    })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    // 模拟后端落库后响应式字段生效：body_translated 从 null → 文本
    release.body_translated = '译文'
    await nextTick()

    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(onTranslated).toHaveBeenCalledOnce()
  })

  it('body_translated 已有值时变化不触发 onTranslated（仅无→有）', async () => {
    translateReleaseMock.mockResolvedValue(undefined)
    const release = reactive(makeRelease({ body_translated: '已有译文' }))
    const onTranslated = vi.fn()
    const wrapper = mountHarness({
      release: () => release,
      onTranslated,
    })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    release.body_translated = '新译文'
    await nextTick()

    // 已有值→新值：不属于「从无到有」，不触发 onTranslated（复位依赖列表刷新）
    expect(onTranslated).not.toHaveBeenCalled()
  })
})

describe('useReleaseTranslate — 流式分片', () => {
  it('stream 开启时订阅事件，分片合帧累加进 streamText', async () => {
    translateReleaseMock.mockImplementation(() => deferred().promise)
    const release = reactive(makeRelease({ id: 7 }))
    const wrapper = mountHarness({ release: () => release, stream: true })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    expect(events.releaseTranslateChunk.listen).toHaveBeenCalledOnce()
    emitChunk(7, '你好')
    emitChunk(7, '世界')
    // 合帧窗口未到：分片还在队列里（逐片上屏会让每片都触发一次全量 Markdown 重解析）
    expect(wrapper.get('.stream').text()).toBe('')

    await waitFrame()
    expect(wrapper.get('.stream').text()).toBe('你好世界')
  })

  it('stream 未开启时不订阅（卡片列表不订阅，避免每行一个监听器）', async () => {
    translateReleaseMock.mockResolvedValue(undefined)
    const wrapper = mountHarness({ release: () => makeRelease({ id: 7 }) })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    expect(events.releaseTranslateChunk.listen).not.toHaveBeenCalled()
  })

  it('其它 release 的分片不入本条的缓冲（并行翻译时事件会混流）', async () => {
    translateReleaseMock.mockImplementation(() => deferred().promise)
    const release = reactive(makeRelease({ id: 7 }))
    const wrapper = mountHarness({ release: () => release, stream: true })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    emitChunk(99, '别条的译文')
    await waitFrame()

    expect(wrapper.get('.stream').text()).toBe('')
  })

  it('翻译失败：保留已显示的部分文本，未合帧的尾巴也一并上屏', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease({ id: 7 }))
    const showToast = vi.fn()
    const wrapper = mountHarness({ release: () => release, stream: true, showToast })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    emitChunk(7, '半截译')
    emitChunk(7, '文')
    cmd.reject(new Error('boom'))
    // 不等合帧窗口：失败收尾必须把未处理的分片落盘，
    // 否则最后几十毫秒的内容会凭空消失
    await flushPromises()

    expect(wrapper.get('.stream').text()).toBe('半截译文')
    expect(showToast).toHaveBeenCalledWith(t('release.translate_failed') + 'boom')
    expect(wrapper.find('.busy').exists()).toBe(false)
  })

  it('失败收尾后退订，之后到达的分片不再写入', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease({ id: 7 }))
    const wrapper = mountHarness({ release: () => release, stream: true, showToast: vi.fn() })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    emitChunk(7, '半截')
    cmd.reject(new Error('boom'))
    await flushPromises()
    emitChunk(7, '迟到的尾巴')
    await waitFrame()

    expect(wrapper.get('.stream').text()).toBe('半截')
  })

  it('译文落库（body_translated 从无到有）后清空缓冲，避免与 props 重复显示', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease({ id: 7 }))
    const wrapper = mountHarness({ release: () => release, stream: true })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    emitChunk(7, '译文')
    await waitFrame()
    expect(wrapper.get('.stream').text()).toBe('译文')

    // 命令成功但列表尚未刷新：缓冲留着兜住这段空窗（避免闪一下空白）
    cmd.resolve()
    await flushPromises()
    expect(wrapper.get('.stream').text()).toBe('译文')

    release.body_translated = '译文全文'
    await nextTick()

    expect(wrapper.get('.stream').text()).toBe('')
  })

  it('翻译进行中重复触发：只发一次命令、只订阅一次（监听器不被覆盖）', async () => {
    const cmd = deferred()
    translateReleaseMock.mockImplementation(() => cmd.promise)
    const release = reactive(makeRelease({ id: 7 }))
    const wrapper = mountHarness({ release: () => release, stream: true })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()
    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    expect(translateReleaseMock).toHaveBeenCalledTimes(1)
    expect(events.releaseTranslateChunk.listen).toHaveBeenCalledOnce()

    cmd.resolve()
    await flushPromises()
  })

  it('翻到另一个版本时清空缓冲（缓冲属于上一条）', async () => {
    translateReleaseMock.mockImplementation(() => deferred().promise)
    const release = reactive(makeRelease({ id: 7 }))
    const wrapper = mountHarness({ release: () => release, stream: true })

    await wrapper.get('.translate').trigger('click')
    await flushPromises()

    emitChunk(7, '上一条的译文')
    await waitFrame()
    expect(wrapper.get('.stream').text()).toBe('上一条的译文')

    release.id = 8
    await nextTick()

    expect(wrapper.get('.stream').text()).toBe('')
  })
})
