import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { t } from '../i18n'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  message: vi.fn(),
  confirm: vi.fn(),
}))

vi.mock('../api/logs', () => ({
  searchLogs: vi.fn(),
  clearLogs: vi.fn(),
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

function pageInputValue(wrapper: Awaited<ReturnType<typeof mountLogTab>>): string {
  return (wrapper.find('.pagination-input').element as HTMLInputElement).value
}

function searchInputValue(wrapper: Awaited<ReturnType<typeof mountLogTab>>): string {
  return (wrapper.find('.search-input').element as HTMLInputElement).value
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
})

afterEach(() => {
  vi.useRealTimers()
})

/**
 * LogTab.vue 真实运行场景测试（真实 i18n / utils / translateError）
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

    expect(wrapper.text()).toContain(t('log.no_records'))
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

  it('搜索时回到第 1 页（DOM 入口：先翻页再搜索）', async () => {
    // 初始即 120 条（3 页），mount 后分页栏存在
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )
    const wrapper = await mountLogTab()

    // 翻到第 2 页
    await wrapper.findAll('.pagination-btn')[1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))
    expect(pageInputValue(wrapper)).toBe('2')

    // 输入搜索词触发回第 1 页（搜索后仍有多页数据，分页栏保留）
    vi.mocked(searchLogs).mockClear()
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )
    await wrapper.find('input').setValue('test')
    await new Promise(resolve => setTimeout(resolve, 350))

    expect(pageInputValue(wrapper)).toBe('1')
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

    expect(searchInputValue(wrapper)).toBe('')
    expect(searchLogs).toHaveBeenCalled()
  })
})

describe('LogTab.vue — 级别筛选', () => {
  it('默认筛选 all（显示「全部」）', async () => {
    const wrapper = await mountLogTab()

    expect(wrapper.find('.filter-value').text()).toBe(t('log.filter_all'))
  })

  it('点击触发按钮切换筛选 dropdown', async () => {
    const wrapper = await mountLogTab()

    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(true)

    await wrapper.find('.filter-trigger').trigger('click')
    expect(wrapper.find('.filter-dropdown').exists()).toBe(false)
  })

  it('选择 ERROR 级别后使用 level 参数重新查询并更新显示', async () => {
    vi.mocked(searchLogs).mockResolvedValue(createSearchResult([], 0))
    const wrapper = await mountLogTab()
    vi.mocked(searchLogs).mockClear()

    await wrapper.find('.filter-trigger').trigger('click')
    const buttons = wrapper.findAll('.filter-dropdown button')
    // 最后一个按钮是 ERROR
    await buttons[buttons.length - 1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(wrapper.find('.filter-value').text()).toBe('ERROR')
    expect(searchLogs).toHaveBeenCalledWith('', 1, 50, 'ERROR')
  })

  it('切换级别后回到第 1 页', async () => {
    // 初始即 120 条（3 页），mount 后分页栏存在
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )
    const wrapper = await mountLogTab()

    // 翻到第 2 页
    await wrapper.findAll('.pagination-btn')[1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))
    expect(pageInputValue(wrapper)).toBe('2')

    await wrapper.find('.filter-trigger').trigger('click')
    await wrapper.findAll('.filter-dropdown button')[1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(pageInputValue(wrapper)).toBe('1')
  })
})

describe('LogTab.vue — 分页', () => {
  it('超过 pageSize 时显示分页（3 页）', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )

    const wrapper = await mountLogTab()

    expect(wrapper.find('.pagination').exists()).toBe(true)
    expect(wrapper.find('.pagination-info').text()).toContain('/ 3')
  })

  it('不超过 pageSize 时不显示分页', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 30 }, (_, i) => createLogEntry({ id: i + 1 })), 30)
    )

    const wrapper = await mountLogTab()

    expect(wrapper.find('.pagination').exists()).toBe(false)
  })

  it('点击下一页后页码更新', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 1 })), 120)
    )
    const wrapper = await mountLogTab()

    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult(Array.from({ length: 50 }, (_, i) => createLogEntry({ id: i + 51 })), 120)
    )
    await wrapper.findAll('.pagination-btn')[1].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(pageInputValue(wrapper)).toBe('2')
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

    const pageInput = wrapper.find('.pagination-input')
    await pageInput.setValue('2')
    await pageInput.trigger('keyup.enter')
    await new Promise(resolve => setTimeout(resolve, 20))

    expect(pageInputValue(wrapper)).toBe('2')
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

  it('message_key + message_args 走 tm 翻译渲染（未知 key 原样显示）', async () => {
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

    // 真实 tm：未知 key 原样返回，且 {owner}/{repo} 占位符被替换
    expect(wrapper.text()).toContain('source.check.in_progress')
  })

  it('message_args 中 error 以 err. 开头时经真实 translateError 翻译', async () => {
    vi.mocked(searchLogs).mockResolvedValue(
      createSearchResult([{
        id: 1,
        level: 'ERROR',
        message: 'Connection failed',
        created_at: '2025-06-01T12:00:00Z',
        message_key: 'check.failed',
        message_args: JSON.stringify({ owner: 'tauri-apps', repo: 'tauri', error: 'err.repo_not_found' }),
        rendered_message: null,
      }], 1)
    )

    const wrapper = await mountLogTab()

    // translateError('err.repo_not_found') → '不存在该仓库'，注入 tm 渲染
    expect(wrapper.text()).toContain('检查 tauri-apps/tauri 失败: 不存在该仓库')
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
    await wrapper.setProps({ refreshKey: 1 } as never)
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
    await wrapper.setProps({ refreshKey: 1 } as never)
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

describe('LogTab.vue — 错误处理', () => {
  it('searchLogs 抛错时清空列表并显示空状态（无未捕获 rejection）', async () => {
    vi.mocked(searchLogs).mockRejectedValueOnce(new Error('db error'))
    const wrapper = await mountLogTab()
    await flushPromises()

    // 错误已被 catch，列表为空，显示空状态提示
    expect(wrapper.findAll('.log-item')).toHaveLength(0)
    expect(wrapper.find('.empty').text()).toContain(t('log.no_records'))
  })
})

describe('LogTab.vue — 定时器清理', () => {
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
