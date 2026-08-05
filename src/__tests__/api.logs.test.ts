import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../api/client', () => ({
  invokeI18n: vi.fn(),
}))

import { invokeI18n } from '../api/client'
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
    vi.mocked(invokeI18n).mockResolvedValue(result)

    const out = await searchLogs('boom', 2, 20, 'error')

    expect(invokeI18n).toHaveBeenCalledWith('search_logs', {
      keyword: 'boom',
      page: 2,
      pageSize: 20,
      level: 'error',
    })
    expect(out).toBe(result)
  })

  it('不带 level 时 level 传 undefined', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(createSearchResult())

    await searchLogs('', 1, 50)

    expect(invokeI18n).toHaveBeenCalledWith('search_logs', {
      keyword: '',
      page: 1,
      pageSize: 50,
      level: undefined,
    })
  })

  it('invokeI18n 失败时错误原样冒泡', async () => {
    vi.mocked(invokeI18n).mockRejectedValue(new Error('Error: err.db_query_failed'))

    await expect(searchLogs('', 1, 50)).rejects.toThrow('err.db_query_failed')
  })
})

describe('clearLogs', () => {
  it('调起 clear_logs 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await clearLogs()

    expect(invokeI18n).toHaveBeenCalledWith('clear_logs')
  })

  it('invokeI18n 失败时错误原样冒泡', async () => {
    vi.mocked(invokeI18n).mockRejectedValue(new Error('Error: err.clear_failed'))

    await expect(clearLogs()).rejects.toThrow('err.clear_failed')
  })
})
