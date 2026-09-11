import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

import { invoke } from '@tauri-apps/api/core'
import { setLocale } from '../i18n'
import {
  installGlobalErrorHandlers,
  reportFrontendError,
  resetReportThrottle,
  setErrorToastSink,
} from '../api/report-error'

/** 取出 invoke 收到的 (命令名, 参数)，避免每个用例重复解构。 */
function lastInvokeCall(): [string, { messageKey: string; detail: string; info: string | null }] {
  const calls = vi.mocked(invoke).mock.calls
  expect(calls.length).toBeGreaterThan(0)
  return calls[calls.length - 1] as never
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(invoke).mockResolvedValue(null)
  resetReportThrottle()
  setErrorToastSink(() => {})
  setLocale('zh-CN')
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('reportFrontendError', () => {
  it('把错误落库到 report_frontend_error（含截断后的明细）', async () => {
    await reportFrontendError('ui.unhandled_rejection', new Error('boom'), 'setup')

    const [cmd, args] = lastInvokeCall()
    expect(cmd).toBe('report_frontend_error')
    expect(args.messageKey).toBe('ui.unhandled_rejection')
    expect(args.detail).toContain('boom')
    expect(args.info).toBe('setup')
  })

  it('同一 key 在节流窗口内只上报一条，窗口过后可再报', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-09-11T00:00:00Z'))

    await reportFrontendError('ui.vue_error', new Error('a'))
    await reportFrontendError('ui.vue_error', new Error('b'))
    expect(vi.mocked(invoke).mock.calls.length).toBe(1)

    // 不同 key 不受同 key 节流影响
    await reportFrontendError('ui.window_error', new Error('c'))
    expect(vi.mocked(invoke).mock.calls.length).toBe(2)

    vi.setSystemTime(new Date('2026-09-11T00:01:01Z'))
    await reportFrontendError('ui.vue_error', new Error('d'))
    expect(vi.mocked(invoke).mock.calls.length).toBe(3)
    expect(lastInvokeCall()[1].detail).toContain('d')
  })

  it('超长堆栈被截断到 2048 字符以内', async () => {
    const long = new Error('x'.repeat(5000))
    await reportFrontendError('ui.vue_error', long)

    const { detail } = lastInvokeCall()[1]
    expect(detail.length).toBeLessThanOrEqual(2049)
    expect(detail.endsWith('…')).toBe(true)
  })

  it('toast 文案复用日志模板并填入明细（不出现裸占位符）', async () => {
    const toast = vi.fn()
    setErrorToastSink(toast)

    await reportFrontendError('ui.unhandled_rejection', new Error('boom'))

    expect(toast).toHaveBeenCalledTimes(1)
    const msg = toast.mock.calls[0][0] as string
    expect(msg).not.toContain('{error}')
    expect(msg).toContain('boom')
  })

  it('被节流时不重复弹 toast', async () => {
    const toast = vi.fn()
    setErrorToastSink(toast)

    await reportFrontendError('ui.vue_error', new Error('a'))
    await reportFrontendError('ui.vue_error', new Error('b'))

    expect(toast).toHaveBeenCalledTimes(1)
  })

  it('上报失败（IPC 拒绝）不抛错、不递归', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('err.db_connect|locked'))

    await expect(reportFrontendError('ui.vue_error', new Error('boom'))).resolves.toBeUndefined()
  })

  it('toast 出口自身抛错不影响上报', async () => {
    setErrorToastSink(() => {
      throw new Error('toast crashed')
    })

    await reportFrontendError('ui.vue_error', new Error('boom'))

    expect(lastInvokeCall()[0]).toBe('report_frontend_error')
  })

  it('非 Error 的 reject 值（字符串 / 对象）也能落库', async () => {
    await reportFrontendError('ui.unhandled_rejection', 'plain reason')
    expect(lastInvokeCall()[1].detail).toBe('plain reason')

    resetReportThrottle()
    await reportFrontendError('ui.unhandled_rejection', { code: 500 })
    expect(lastInvokeCall()[1].detail).toBe('{"code":500}')
  })
})

describe('installGlobalErrorHandlers', () => {
  /** 构造带自定义字段的事件（jsdom 的 PromiseRejectionEvent/ErrorEvent 覆盖不一，故手工挂字段）。 */
  function dispatch(type: string, props: Record<string, unknown> = {}): void {
    const event = new Event(type)
    Object.assign(event, props)
    window.dispatchEvent(event)
  }

  it('注册 Vue errorHandler 并把渲染错误上报为 ui.vue_error', async () => {
    const app = { config: { errorHandler: undefined } } as never
    installGlobalErrorHandlers(app)

    const handler = (app as { config: { errorHandler?: (e: unknown, i: unknown, info: string) => void } })
      .config.errorHandler
    expect(typeof handler).toBe('function')

    handler!(new Error('render boom'), null, 'render function')
    await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.length).toBe(1))

    const [, args] = lastInvokeCall()
    expect(args.messageKey).toBe('ui.vue_error')
    expect(args.detail).toContain('render boom')
    expect(args.info).toBe('render function')
  })

  it('安装 errorHandler 后仍在 DEV 控制台打印堆栈（不得吞掉 Vue 的原始输出）', async () => {
    // 回归护栅：Vue 的 handleError 一旦发现 errorHandler 存在就 callWithErrorHandling → return，
    // 不再执行 logError（dev 下它负责 warn + throw err，是 devtools 里堆栈的唯一来源）。
    // 若本模块不自己 console.error，`pnpm tauri dev` 调试时错误就只剩 toast/日志页。
    expect(import.meta.env.DEV).toBe(true) // 若测试构下不再是 DEV，本用例需改写
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

    const app = { config: { errorHandler: undefined } } as never
    installGlobalErrorHandlers(app)
    ;(app as { config: { errorHandler: (e: unknown, i: unknown, info: string) => void } })
      .config.errorHandler(new Error('render boom'), null, 'render function')

    expect(errSpy).toHaveBeenCalledTimes(1)
    const [prefix, logged] = errSpy.mock.calls[0] as [string, unknown]
    expect(prefix).toContain('render function') // 保留 Vue 的「哪个阶段出错」信息
    expect((logged as Error).message).toBe('render boom')
    errSpy.mockRestore()
  })

  it('DEV 下 console.error 不参与节流（同一错误风暴仍逐次打印，便于调试）', async () => {
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    const app = { config: { errorHandler: undefined } } as never
    installGlobalErrorHandlers(app)
    const handler = (app as { config: { errorHandler: (e: unknown, i: unknown, info: string) => void } })
      .config.errorHandler

    handler(new Error('a'), null, 'render')
    handler(new Error('b'), null, 'render')

    expect(errSpy).toHaveBeenCalledTimes(2)
    await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.length).toBe(1)) // 落库仍节流
    errSpy.mockRestore()
  })

  it('unhandledrejection 上报为 ui.unhandled_rejection', async () => {
    installGlobalErrorHandlers({ config: {} } as never)

    dispatch('unhandledrejection', { reason: new Error('async boom') })
    await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.length).toBe(1))

    const [, args] = lastInvokeCall()
    expect(args.messageKey).toBe('ui.unhandled_rejection')
    expect(args.detail).toContain('async boom')
  })

  it('window error 上报为 ui.window_error；无 error 字段的资源加载失败不上报', async () => {
    installGlobalErrorHandlers({ config: {} } as never)

    // 资源加载失败（<img>/<script>）：error 为 null → 不当作未处理异常
    dispatch('error', { error: null })
    await Promise.resolve()
    expect(vi.mocked(invoke).mock.calls.length).toBe(0)

    dispatch('error', { error: new Error('sync boom') })
    await vi.waitFor(() => expect(vi.mocked(invoke).mock.calls.length).toBe(1))
    expect(lastInvokeCall()[1].messageKey).toBe('ui.window_error')
  })
})
