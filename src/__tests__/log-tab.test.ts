import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  message: vi.fn(),
  confirm: vi.fn(),
}))

vi.mock('../api/logs', () => ({
  searchLogs: vi.fn(),
  clearLogs: vi.fn(),
}))

vi.mock('../i18n', () => ({
  t: vi.fn((key: string, ...args: string[]) => args.length ? `${key}:${args.join(',')}` : key),
  tm: vi.fn((key: string, args: Record<string, string>) => {
    // 模拟真实 tm：先 mock 一个"翻译文本"然后替换 {key} 占位符
    let text = key
    Object.entries(args).forEach(([k, v]) => {
      text = text.replace(`{${k}}`, v)
    })
    return `${key}(${Object.values(args).join(',')})`
  }),
}))

vi.mock('../api/client', () => ({
  translateError: vi.fn((raw: string) => {
    if (raw.startsWith('err.')) return raw.replace('err.', '错误:')
    return raw
  }),
}))

vi.mock('../utils', () => ({
  formatDate: vi.fn(() => '2025-06-01 12:00'),
  logLevelClass: vi.fn((level: string) => {
    if (level === 'ERROR') return 'log-error'
    if (level === 'WARN') return 'log-warn'
    return 'log-info'
  }),
}))

import LogTab from '../components/LogTab.vue'
import { searchLogs, clearLogs } from '../api/logs'
import { confirm } from '@tauri-apps/plugin-dialog'

function createLogEntry(overrides: Record<string, unknown> = {}) {
  return {
    id: 1,
    level: 'INFO',
    message: 'Source check completed',
    created_at: '2025-06-01T12:00:00Z',
    message_key: null,
    message_args: null,
    rendered_message: null,
    ...overrides,
  }
}

function createSearchResult(entries: any[] = [], total = 0) {
  return { entries, total, page: 1, page_size: 50 }
}

async function mountLogTab(props: Record<string, unknown> = {}) {
  const wrapper = mount(LogTab, {
    props: { refreshKey: 0, ...props },
  })
  // 等待 onMounted 中的 loadData 完成
  await flushPromises()
  await new Promise(resolve => setTimeout(resolve, 20))
  return wrapper
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
})

/**
 * LogTab.vue 真实运行场景测试
 *
 * 日志查询组件，提供：
 * - 挂载时自动加载
 * - 搜索（300ms 防抖）
 * - 级别筛选 dropdown
 * - 分页浏览
 * - 清空日志（确认对话框）
 * - 外部刷新（refreshKey prop）
 */
describe('LogTab.vue — 挂载与加载', () => {
  it('挂载时自动调用 searchLogs', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ id: 1 }), createLogEntry({ id: 2 })], 2)
    )

    const wrapper = await mountLogTab()

    expect(searchLogs).toHaveBeenCalledWith('', 1, 50, undefined)
    expect(wrapper.findAll('.log-item')).toHaveLength(2)
  })

  it('无日志时显示空状态', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))

    const wrapper = await mountLogTab()

    expect(wrapper.text()).toContain('log.no_records')
  })

  it('有日志时渲染条目', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ id: 1, level: 'INFO', message: 'OK' })], 1)
    )

    const wrapper = await mountLogTab()

    expect(wrapper.text()).toContain('OK')
    expect(wrapper.text()).toContain('INFO')
  })
})

describe('LogTab.vue — 搜索', () => {
  it('输入搜索词后触发防抖（300ms）', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    await wrapper.find('input').setValue('error')
    // debounce timer pending → 尚未调用
    expect(searchLogs).not.toHaveBeenCalled()

    // 等待 debounce
    await new Promise(resolve => setTimeout(resolve, 350))

    expect(searchLogs).toHaveBeenCalled()
  })

  it('搜索时回到第 1 页', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    ;(wrapper.vm as any).currentPage = 3
    vi.mocked(searchLogs).mockClear()

    await wrapper.find('input').setValue('test')
    await new Promise(resolve => setTimeout(resolve, 350))

    expect((wrapper.vm as any).currentPage).toBe(1)
  })

  it('有搜索词时显示清空按钮', async () => {
    const wrapper = await mountLogTab()

    expect(wrapper.find('.input-clear-btn').exists()).toBe(false)

    await wrapper.find('input').setValue('test')
    await new Promise(resolve => setTimeout(resolve, 50))

    expect(wrapper.find('.input-clear-btn').exists()).toBe(true)
  })

  it('点击清空按钮清除搜索词并重新加载', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    // 先输入搜索词触发加载
    await wrapper.find('input').setValue('test')
    await new Promise(resolve => setTimeout(resolve, 350))
    vi.mocked(searchLogs).mockClear()

    // 点击清空
    await wrapper.find('.input-clear-btn').trigger('click')

    expect((wrapper.vm as any).searchKeyword).toBe('')
    expect(searchLogs).toHaveBeenCalled()
  })
})

describe('LogTab.vue — 级别筛选', () => {
  it('默认筛选 all', async () => {
    const wrapper = await mountLogTab()

    expect((wrapper.vm as any).levelFilter).toBe('all')
  })

  it('点击触发按钮切换筛选 dropdown', async () => {
    const wrapper = await mountLogTab()

    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(true)

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择 ERROR 级别后使用 level 参数重新查询', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    await wrapper.find('.filter-trigger').trigger('click')
    const buttons = wrapper.findAll('.filter-dropdown button')
    // 最后一个按钮是 ERROR
    await buttons[buttons.length - 1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect((wrapper.vm as any).levelFilter).toBe('ERROR')
    expect(searchLogs).toHaveBeenCalledWith('', 1, 50, 'ERROR')
  })

  it('切换级别后回到第 1 页', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    ;(wrapper.vm as any).currentPage = 3

    await wrapper.find('.filter-trigger').trigger('click')
    await wrapper.findAll('.filter-dropdown button')[1].trigger('click')

    expect((wrapper.vm as any).currentPage).toBe(1)
  })
})

describe('LogTab.vue — 分页', () => {
  it('超过 pageSize 时显示分页', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )

    const wrapper = await mountLogTab()

    expect((wrapper.vm as any).totalPages).toBe(3)
    expect(wrapper.find('.pagination').exists()).toBe(true)
    expect(wrapper.text()).toContain('/ 3')
  })

  it('不超过 pageSize 时不显示分页', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 30 }, (_, i) => createLogEntry({ id: i + 1 })), 30)
    )

    const wrapper = await mountLogTab()

    expect((wrapper.vm as any).totalPages).toBe(1)
    expect(wrapper.find('.pagination').exists()).toBe(false)
  })

  it('点击下一页', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 51 })), 120)
    )
    await wrapper.findAll('.pagination-btn')[1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect((wrapper.vm as any).currentPage).toBe(2)
  })

  it('第 1 页时上一页按钮禁用', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )

    const wrapper = await mountLogTab()

    const prevBtn = wrapper.findAll('.pagination-btn')[0]
    expect((prevBtn.element as HTMLButtonElement).disabled).toBe(true)
  })

  it('输入页码后按 Enter 跳转', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    const pageInput = wrapper.find('.pagination-input')
    await pageInput.setValue('2')
    await pageInput.trigger('keyup.enter')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect((wrapper.vm as any).currentPage).toBe(2)
  })
})

describe('LogTab.vue — 清空日志', () => {
  it('点击清空按钮弹出确认对话框', async () => {
    vi.mocked(confirm).mockResolvedValue(true)
    vi.mocked(clearLogs).mockResolvedValue(undefined)

    const wrapper = await mountLogTab()

    await wrapper.find('.btn-icon').trigger('click')

    expect(confirm).toHaveBeenCalled()
  })

  it('确认后调用 clearLogs 并刷新', async () => {
    vi.mocked(confirm).mockResolvedValue(true)
    vi.mocked(clearLogs).mockResolvedValue(undefined)
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))

    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    await wrapper.find('.btn-icon').trigger('click')
    await flushPromises()
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(clearLogs).toHaveBeenCalled()
    expect(searchLogs).toHaveBeenCalled()
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('取消确认不清空', async () => {
    vi.mocked(confirm).mockResolvedValue(false)

    const wrapper = await mountLogTab()

    await wrapper.find('.btn-icon').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(clearLogs).not.toHaveBeenCalled()
  })
})

describe('LogTab.vue — renderMessage', () => {
  it('普通日志直接显示 message', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ message: 'Hello world' })], 1)
    )

    const wrapper = await mountLogTab()

    expect(wrapper.text()).toContain('Hello world')
  })

  it('message_key + message_args 使用 tm 翻译', async () => {
    const { tm } = await import('../i18n')
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([{
        id: 1,
        level: 'INFO',
        message: 'source.check.in_progress',
        created_at: '2025-06-01T12:00:00Z',
        message_key: 'source.check.in_progress',
        message_args: JSON.stringify({ owner: 'tauri-apps', repo: 'tauri' }),
        rendered_message: null,
      }], 1)
    )

    const wrapper = await mountLogTab()

    // tm 被调用时传入了 message_key 和解析后的 args
    expect(tm).toHaveBeenCalledWith('source.check.in_progress', {
      owner: 'tauri-apps',
      repo: 'tauri',
    })
  })

  it('message_args 中 error 以 err. 开头时翻译', async () => {
    const { translateError } = await import('../api/client')
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([{
        id: 1,
        level: 'ERROR',
        message: 'Connection failed',
        created_at: '2025-06-01T12:00:00Z',
        message_key: 'source.check.failed',
        message_args: JSON.stringify({ error: 'err.network.timeout' }),
        rendered_message: null,
      }], 1)
    )

    await mountLogTab()

    // translateError 被调用，且结果被注入到 args.error 中传给 tm
    expect(translateError).toHaveBeenCalledWith('err.network.timeout')
  })

  it('JSON.parse 异常时降级显示原始 message', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({
        message_key: 'some.key',
        message_args: 'invalid json{',
        message: 'fallback msg',
      })], 1)
    )

    const wrapper = await mountLogTab()

    expect(wrapper.text()).toContain('fallback msg')
  })
})

describe('LogTab.vue — 外部刷新', () => {
  it('refreshKey 变化时重新加载', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ id: 99, message: 'New' })], 1)
    )
    await (wrapper as any).setProps({ refreshKey: 1 })
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(searchLogs).toHaveBeenCalled()
  })
})

describe('LogTab.vue — 日志级别样式', () => {
  it('ERROR 级别有 log-error class', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ level: 'ERROR' })], 1)
    )
    const wrapper = await mountLogTab()

    expect(wrapper.find('.log-level').classes()).toContain('log-error')
  })

  it('WARN 级别有 log-warn class', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ level: 'WARN' })], 1)
    )
    const wrapper = await mountLogTab()

    expect(wrapper.find('.log-level').classes()).toContain('log-warn')
  })

  it('INFO 级别有 log-info class', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([createLogEntry({ level: 'INFO' })], 1)
    )
    const wrapper = await mountLogTab()

    expect(wrapper.find('.log-level').classes()).toContain('log-info')
  })
})

describe('LogTab.vue — 并发竞态保护', () => {
  it('并发请求时丢弃陈旧响应，保留最新结果', async () => {
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    // 用可控的 pending Promise 模拟两次并发请求：旧请求慢、新请求快
    let resolveFirst!: (v: ReturnType<typeof createSearchResult>) => void
    let resolveSecond!: (v: ReturnType<typeof createSearchResult>) => void
    const firstPromise = new Promise<ReturnType<typeof createSearchResult>>(r => { resolveFirst = r })
    const secondPromise = new Promise<ReturnType<typeof createSearchResult>>(r => { resolveSecond = r })
    vi.mocked(searchLogs).mockReturnValueOnce(firstPromise)
    vi.mocked(searchLogs).mockReturnValueOnce(secondPromise)

    // 触发第一次 loadData（旧请求，将慢返回）
    await wrapper.find('.filter-trigger').trigger('click')
    await wrapper.findAll('.filter-dropdown button')[3].trigger('click') // ERROR
    // 触发第二次 loadData（新请求，将先返回）
    await (wrapper as any).setProps({ refreshKey: 1 })
    await flushPromises() // 等待 watch 回调触发第二次 loadData

    // 新请求先解析
    resolveSecond(createSearchResult([createLogEntry({ id: 2, message: 'NEW_DATA' })], 1))
    await flushPromises()
    // 旧请求后解析（应被丢弃，不覆盖新结果）
    resolveFirst(createSearchResult([createLogEntry({ id: 1, message: 'OLD_DATA' })], 1))
    await flushPromises()

    expect(wrapper.text()).toContain('NEW_DATA')
    expect(wrapper.text()).not.toContain('OLD_DATA')
  })
})

describe('LogTab.vue — 错误处理（P1 #5）', () => {
  it('searchLogs 抛错时清空列表、复位 loading 且无未捕获 rejection', async () => {
    vi.mocked(searchLogs).mockRejectedValueOnce(new Error('db error'))
    const wrapper = await mountLogTab()
    await flushPromises()

    // 错误已被 catch，loading 复位，列表为空
    expect((wrapper.vm as any).loading).toBe(false)
    expect((wrapper.vm as any).logs).toEqual([])
    expect((wrapper.vm as any).totalLogs).toBe(0)
    expect(wrapper.text()).toContain('log.no_records')
  })
})

describe('LogTab.vue — 定时器清理（P1 #10）', () => {
  it('卸载时清理 debounce 定时器，不再触发后续 loadData', async () => {
    vi.useFakeTimers()
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    // 不使用 mountLogTab helper（其内部 setTimeout 与 fake timers 不兼容），直接 mount
    const wrapper = mount(LogTab, { props: { refreshKey: 0 } })
    await flushPromises()
    vi.advanceTimersByTime(20) // 推进 onMounted 中可能的微任务等待
    vi.mocked(searchLogs).mockClear()

    // 输入搜索词，启动 debounce 定时器（300ms）
    await wrapper.find('input').setValue('test')
    await flushPromises()
    // 立即卸载，应清理定时器
    wrapper.unmount()
    // 推进超过 debounce 时间，定时器已被清理，不应再调用 searchLogs
    vi.advanceTimersByTime(400)
    await flushPromises()

    expect(searchLogs).not.toHaveBeenCalled()
    vi.useRealTimers()
  })
})
