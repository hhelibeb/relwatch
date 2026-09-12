import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { useBilibiliLogin } from '../composables/useBilibiliLogin'
import { InvokeI18nError } from '../api/client'
import {
  readBilibiliLoginCookie,
  openBilibiliLoginWindow,
  closeBilibiliLoginWindow,
  setCredential,
} from '../api/settings'

vi.mock('../api/settings', () => ({
  readBilibiliLoginCookie: vi.fn(),
  openBilibiliLoginWindow: vi.fn().mockResolvedValue(undefined),
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
const openBilibiliLoginWindowMock = vi.mocked(openBilibiliLoginWindow)
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
  readBilibiliLoginCookieMock.mockRejectedValue(new InvokeI18nError('err.bili_login_window_missing', [], 'window missing'))
  openBilibiliLoginWindowMock.mockResolvedValue(undefined)
  closeBilibiliLoginWindowMock.mockResolvedValue(undefined)
  setCredentialMock.mockResolvedValue(undefined)
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useBilibiliLogin — 登录状态机', () => {
  it('窗口缺失时经 Rust 建窗并进入轮询（busy 置位）', async () => {
    const { wrapper } = mountHarness()

    const p = wrapper.get('.login').trigger('click')
    await nextTick()

    // 探测窗口缺失 → 调用 Rust 建窗命令（传入窗口标题）
    expect(openBilibiliLoginWindowMock).toHaveBeenCalledWith(t('settings.bilibili_login_title'))
    expect(wrapper.find('.busy').exists()).toBe(true)
    // 命令 resolve 后进入轮询
    await p
    expect(readBilibiliLoginCookieMock).toHaveBeenCalled()
  })

  it('窗口创建失败：提示失败并复位 busy', async () => {
    openBilibiliLoginWindowMock.mockRejectedValue(new Error('create failed'))
    const { wrapper, showToast } = mountHarness()

    const p = wrapper.get('.login').trigger('click')
    await p
    await nextTick()

    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(showToast).toHaveBeenCalledWith(t('settings.bilibili_login_window_failed'))
  })

  it('建窗命令报成功但窗口实际未建成：探活失败即解锁按钮，不卡在等待登录', async () => {
    // 复现真实缺陷：tauri-runtime-wry 事件循环对 webview 创建失败只 log::error!
    // 且不回传，build() 仍返回 Ok → 窗口闪退、但命令 resolve。此前前端据此转入
    // 轮询，两个定时器都未归位（轮询遇 window_missing 才重置 busy，而探活已证明
    // 窗口不在）→ 按钮永久显示“等待登录”，无法重试。
    // 建窗命令始终 resolve；随后任何一次读取都报窗口缺失。
    readBilibiliLoginCookieMock.mockRejectedValue(
      new InvokeI18nError('err.bili_login_window_missing', [], 'window missing'),
    )
    const { wrapper, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    // 按钮必须解锁（不再 busy）
    expect(wrapper.find('.busy').exists()).toBe(false)
    expect(showToast).toHaveBeenCalledWith(t('settings.bilibili_login_window_failed'))
    // 不得进入轮询：不应留下会反复失败的 2s 定时器
    const callsAfterClick = readBilibiliLoginCookieMock.mock.calls.length
    await vi.advanceTimersByTimeAsync(6000)
    expect(readBilibiliLoginCookieMock.mock.calls.length).toBe(callsAfterClick)
    // 允许重试：再次点击仍会重新建窗（而非被 busy 锁死）
    openBilibiliLoginWindowMock.mockClear()
    await wrapper.get('.login').trigger('click')
    expect(openBilibiliLoginWindowMock).toHaveBeenCalled()
  })

  it('建窗后探活为未登录：正常进入轮询（不误判为建窗失败）', async () => {
    // 预探测：窗口缺失 → 走建窗；建窗后探活：未登录（窗口确实建好了）→ 应继续轮询
    readBilibiliLoginCookieMock
      .mockRejectedValueOnce(new InvokeI18nError('err.bili_login_window_missing', [], 'window missing'))
      .mockRejectedValue(new InvokeI18nError('err.bili_login_not_logged_in', [], 'not logged in'))
    const { wrapper, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    expect(openBilibiliLoginWindowMock).toHaveBeenCalled()
    expect(wrapper.find('.busy').exists()).toBe(true)
    // 未登录不得当成建窗失败
    expect(showToast).not.toHaveBeenCalledWith(t('settings.bilibili_login_window_failed'))
    const callsAfterClick = readBilibiliLoginCookieMock.mock.calls.length
    await vi.advanceTimersByTimeAsync(2000)
    expect(readBilibiliLoginCookieMock.mock.calls.length).toBeGreaterThan(callsAfterClick)
  })

  it('窗口已存在且已登录：不建窗直接收尾', async () => {
    readBilibiliLoginCookieMock.mockResolvedValue(true)
    const { wrapper, onLoginSuccess, showToast } = mountHarness()

    await wrapper.get('.login').trigger('click')
    await nextTick()

    expect(openBilibiliLoginWindowMock).not.toHaveBeenCalled()
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

    expect(openBilibiliLoginWindowMock).not.toHaveBeenCalled()

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
