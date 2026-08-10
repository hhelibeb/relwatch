import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('../api/client', () => ({
  // 直接执行传入的绑定命令函数，使断言落在 invoke 层（命令名+参数与 Rust 端一致）
  invokeI18nFn: vi.fn(async <T>(fn: () => Promise<T>) => fn()),
}))

import { invoke } from '@tauri-apps/api/core'
import { searchLogs, clearLogs } from '../api/logs'
import type { LogSearchResult } from '../api/logs'

beforeEach(() => {
  vi.clearAllMocks()
})

function createSearchResult(overrides: Partial<LogSearchResult> = {}): LogSearchResult {
  return {
    entries: [],
    total: 0,
    page: 1,
    page_size: 50,
    ...overrides,
  }
}

describe('searchLogs', () => {
  it('带 level 时透传全部参数调用 search_logs 命令', async () => {
    const result = createSearchResult({
      entries: [
        {
          id: 1,
          level: 'error',
          message: 'boom',
          created_at: '2025-08-05T00:00:00Z',
          message_key: null,
          message_args: null,
          rendered_message: null,
        },
      ],
      total: 1,
    })
    vi.mocked(invoke).mockResolvedValue(result)

    const out = await searchLogs('boom', 2, 20, 'error')

    expect(invoke).toHaveBeenCalledWith('search_logs', {
      keyword: 'boom',
      page: 2,
      pageSize: 20,
      level: 'error',
    })
    expect(out).toBe(result)
  })

  it('不带 level 时传 null', async () => {
    vi.mocked(invoke).mockResolvedValue(createSearchResult())

    await searchLogs('', 1, 50)

    expect(invoke).toHaveBeenCalledWith('search_logs', {
      keyword: '',
      page: 1,
      pageSize: 50,
      level: null,
    })
  })

  it('invokeI18n 失败时错误原样冒泡', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('Error: err.db_query_failed'))

    await expect(searchLogs('', 1, 50)).rejects.toThrow('err.db_query_failed')
  })
})

describe('clearLogs', () => {
  it('调起 clear_logs 命令', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await clearLogs()

    expect(invoke).toHaveBeenCalledWith('clear_logs')
  })

  it('invokeI18n 失败时错误原样冒泡', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('Error: err.clear_failed'))

    await expect(clearLogs()).rejects.toThrow('err.clear_failed')
  })
})
