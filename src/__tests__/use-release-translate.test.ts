import { describe, expect, it, vi, afterEach } from 'vitest'
import { defineComponent, nextTick, reactive } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'
import { useReleaseTranslate } from '../composables/useReleaseTranslate'
import { translateRelease } from '../api/releases'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'

vi.mock('../api/releases', () => ({
  translateRelease: vi.fn(),
}))

// useUsageTracking 内部直接使用真实实现（track 为 no-op 亦可，避免额外 mock）
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
}) {
  return mount(defineComponent({
    setup() {
      const { translating, handleTranslateRelease } = useReleaseTranslate(opts)
      return { translating, handleTranslateRelease }
    },
    template: `
      <div>
        <button class="translate" @click="handleTranslateRelease">translate</button>
        <span class="busy" v-if="translating">busy</span>
      </div>
    `,
  }))
}

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
