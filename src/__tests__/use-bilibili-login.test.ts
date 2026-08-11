import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { useBilibiliLogin } from '../composables/useBilibiliLogin'
import { InvokeI18nError } from '../api/client'
import {
  readBilibiliLoginCookie,
  closeBilibiliLoginWindow,
  setCredential,
} from '../api/settings'

// WebviewWindow 假实现：记录实例，once 回调可手动触发（tauri://created / tauri://error）
const { MockWebviewWindow } = vi.hoisted(() => {
  class MockWebviewWindow {
    static instances: MockWebviewWindow[] = []
    onceCallbacks: Record<string, () => void> = {}
    constructor(public label: string, public options?: unknown) {
      MockWebviewWindow.instances.push(this)
    }
    once(event: string, cb: () => void) {
      this.onceCallbacks[event] = cb
    }
    fire(event: string) {
      this.onceCallbacks[event]?.()
    }
  }
  return { MockWebviewWindow }
})

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  WebviewWindow: MockWebviewWindow,
}))

vi.mock('../api/settings', () => ({
  readBilibiliLoginCookie: vi.fn(),
  closeBilibiliLoginWindow: vi.fn().mockResolvedValue(undefined),
  setCredential: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('../api/client', async (importOriginal) => {
  const mod = await importOriginal<typeof import('../api/client')>()
  return { ...mod }
})

vi.mock('../composables/useUsageTracking', () => ({
  track: vi.fn(),
}))

import { t } from '../i18n'

const readBilibiliLoginCookieMock = vi.mocked(readBilibiliLoginCookie)
const closeBilibiliLoginWindowMock = vi.mocked(closeBilibiliLoginWindow)
const setCredentialMock = vi.mocked(setCredential)

const Harness = defineComponent({
  props: {
    showToast: { type: Function, required: true },
    onLoginSuccess: { type: Function, required: true },
    onCookieCleared: { type: Function, required: true },
  },
  setup(props) {
    const st = useBilibiliLogin({
      showToast: props.showToast as (msg: string) => void,
      onLoginSuccess: props.onLoginSuccess as () => void,
      onCookieCleared: props.onCookieCleared as () => void,
    })
    return { ...st }
  },
  template: `
    <div>
      <button class="login" @click="handleBilibiliLogin">login</button>
      <button class="clear" @click="handleClearBilibiliCookie">clear</button>
      <span v-if="biliLoginBusy" class="busy">busy</span>
    </div>
  `,
})

function mountHarness() {
  const showToast = vi.fn()
  const onLoginSuccess = vi.fn()
  const onCookieCleared = vi.fn()
  const wrapper = mount(Harness, { props: { showToast, onLoginSuccess, onCookieCleared } })
  return { wrapper, showToast, onLoginSuccess, onCookieCleared }
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.useFakeTimers()
  MockWebviewWindow.instances = []
  readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_window_missing', [], 'window missing'))
  closeBilibiliLoginWindowMock.mockResolvedValue(undefined)
  setCredentialMock.mockResolvedValue(undefined)
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useBilibiliLogin — 登录状态机', () => {
  it('窗口缺失时创建登录窗口并进入轮询（busy 置位）', async () => {
    const { wrapper } = mountHarness()

    const p = wrapper.get('.login').trigger('click')
    await nextTick()

    expect(MockWebviewWindow.instances).toHaveLength(1)
    expect(MockWebviewWindow.instances[0].label).toBe('bilibili-login')
    expect(wrapper.find('.busy').exists()).toBe(true)
    // 等待窗口 created 事件
    MockWebviewWindow.instances[0].fire('tauri://created')
    await p
  })

  it('窗口创建失败：提示失败并复位 busy', async () => {
    const { wrapper, showToast } = mountHarness()

    const p = wrapper.get('.login').trigger('click')
    await nextTick()
    MockWebviewWindow.instances[0].fire('tauri://error')
    await p
    await nextTick()

    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(showToast).toHaveBeenCalledWith(t('settings.bilibili_login_window_failed'))
  })

  it('窗口已存在且已登录：不建窗直接收尾', async () => {
    readBilibiliLoginCookieMock.mockResolvedValue(true)
    const { wrapper, onLoginSuccess, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    expect(MockWebviewWindow.instances).toHaveLength(0)
    expect(onLoginSuccess).toHaveBeenCalledOnce()
    expect(closeBilibiliLoginWindowMock).toHaveBeenCalledWith('bilibili-login')
    expect(showToast).toHaveBeenCalledWith(t('settings.bilibili_login_success'))
  })

  it('轮询中登录成功：停止轮询、关窗、回调与提示', async () => {
    // 窗口已存在且未登录：探测命中 not_logged_in → 不建窗，恢复轮询
    readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_not_logged_in', [], 'not logged in'))
    const { wrapper, onLoginSuccess, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    expect(MockWebviewWindow.instances).toHaveLength(0)

    // 第一次轮询仍未登录
    await vi.advanceTimersByTimeAsync(2000)
    expect(onLoginSuccess).not.toHaveBeenCalled()

    // 第二次轮询登录成功
    readBilibiliLoginCookieMock.mockResolvedValue(true)
    await vi.advanceTimersByTimeAsync(2000)

    expect(onLoginSuccess).toHaveBeenCalledOnce()
    expect(closeBilibiliLoginWindowMock).toHaveBeenCalledWith('bilibili-login')
    expect(showToast).toHaveBeenCalledWith(t('settings.bilibili_login_success'))
    expect(wrapper.find('.busy').exists()).toBe(false)
  })

  it('轮询中窗口被关闭（用户放弃）：停止轮询并复位 busy，不提示失败', async () => {
    readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_not_logged_in', [], 'not logged in'))
    const { wrapper, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_window_missing', [], 'gone'))
    await vi.advanceTimersByTimeAsync(2000)

    expect(wrapper.find('.busy').exists()).toBe(false)
    // 用户主动放弃：不应弹出失败提示
    expect(showToast).not.toHaveBeenCalledWith(expect.stringContaining('bilibili_login_failed'))
  })

  it('60 秒超时：停止轮询、复位 busy（窗口保留可重试）', async () => {
    readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_not_logged_in', [], 'not logged in'))
    const { wrapper } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    await vi.advanceTimersByTimeAsync(60000)

    expect(wrapper.find('.busy').exists()).toBe(false)
    // 超时后不再轮询（读取次数固定为轮询期间次数）
    const callsAfterTimeout = readBilibiliLoginCookieMock.mock.calls.length
    await vi.advanceTimersByTimeAsync(6000)
    expect(readBilibiliLoginCookieMock.mock.calls.length).toBe(callsAfterTimeout)
  })

  it('轮询中持续性错误：停止轮询并提示失败', async () => {
    readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_not_logged_in', [], 'not logged in'))
    const { wrapper, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    readBilibiliLoginCookieMock.mockRejectedValue(new Error('network down'))
    await vi.advanceTimersByTimeAsync(2000)

    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(showToast).toHaveBeenCalledWith(expect.stringContaining('network down'))
  })

  it('清除 Cookie：调用 setCredential 空值并回调', async () => {
    const { wrapper, onCookieCleared, showToast } = mountHarness()

    await wrapper.get('.clear').trigger('click')
    await nextTick()

    expect(setCredentialMock).toHaveBeenCalledWith('bilibili_cookie', '')
    expect(onCookieCleared).toHaveBeenCalledOnce()
    expect(showToast).toHaveBeenCalledWith(t('settings.bilibili_cookie_cleared'))
  })

  it('清除 Cookie 失败：提示保存失败', async () => {
    setCredentialMock.mockRejectedValue(new Error('db locked'))
    const { wrapper, onCookieCleared, showToast } = mountHarness()

    await wrapper.get('.clear').trigger('click')
    await nextTick()

    expect(onCookieCleared).not.toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith(expect.stringContaining('db locked'))
  })
})
