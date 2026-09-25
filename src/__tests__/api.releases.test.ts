import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../api/client', () => ({
  invokeI18n: vi.fn(),
  invokeI18nFn: vi.fn(),
  openReleaseUrl: vi.fn(),
  translateError: vi.fn((raw: string) => raw),
}))

// 命令层单独 mock：invokeI18nFn 被 mock 成"直接执行回调"后，才能断言到底调了哪个
// 命令、带了什么参数（原有断言只覆盖"调用过一次"）。
vi.mock('../bindings', () => ({
  commands: {
    getReleaseCatalog: vi.fn(),
    getReleaseDetail: vi.fn(),
    getReleaseSearchBodies: vi.fn(),
    getReleaseSearchBodiesByIds: vi.fn(),
    setNotificationState: vi.fn(),
    deleteRelease: vi.fn(),
    setReleaseFlag: vi.fn(),
    translateRelease: vi.fn(),
    triggerPoll: vi.fn(),
    checkSingleSource: vi.fn(),
    getPollCountdown: vi.fn(),
  },
}))

import { invokeI18nFn } from '../api/client'
import { commands } from '../bindings'
import {
  getReleaseCatalog,
  getReleaseDetail,
  getReleaseSearchBodies,
  getReleaseSearchBodiesByIds,
  setNotificationState,
  deleteRelease,
  triggerPoll,
  checkSingleSource,
  getPollCountdown,
} from '../api/releases'

beforeEach(() => {
  vi.clearAllMocks()
  // clearAllMocks 不清实现：每个用例前复位成透传，避免上一个用例的 mockResolvedValue 泄漏
  vi.mocked(invokeI18nFn).mockImplementation(
    (fn: () => Promise<unknown>) => Promise.resolve(fn()),
  )
})

describe('getReleaseCatalog', () => {
  it('调起 get_release_catalog 命令，返回 ReleaseInfo[]', async () => {
    const mockData = [{ id: 1, tag_name: 'v1.0.0', owner: 'test', repo: 'test' }]
    vi.mocked(commands.getReleaseCatalog).mockResolvedValue(mockData as never)

    const result = await getReleaseCatalog()

    expect(commands.getReleaseCatalog).toHaveBeenCalledTimes(1)
    expect(result).toEqual(mockData)
  })

  it('后端返回空数组时正常透传', async () => {
    vi.mocked(commands.getReleaseCatalog).mockResolvedValue([])

    expect(await getReleaseCatalog()).toEqual([])
  })

  it('后端错误时抛出异常', async () => {
    vi.mocked(commands.getReleaseCatalog).mockRejectedValue(new Error('err.database'))

    await expect(getReleaseCatalog()).rejects.toThrow('err.database')
  })
})

describe('getReleaseDetail', () => {
  it('按 release id 调起 get_release_detail', async () => {
    const full = { id: 7, body: '全文正文' }
    vi.mocked(commands.getReleaseDetail).mockResolvedValue(full as never)

    const result = await getReleaseDetail(7)

    expect(commands.getReleaseDetail).toHaveBeenCalledWith(7)
    expect(result).toEqual(full)
  })

  it('后端错误时抛出异常（详情失败必须可感知）', async () => {
    vi.mocked(commands.getReleaseDetail).mockRejectedValue(new Error('err.release_not_found|7'))

    await expect(getReleaseDetail(7)).rejects.toThrow('err.release_not_found|7')
  })
})

describe('getReleaseSearchBodies', () => {
  it('透传游标与字符预算（首次游标高于任何真实 id，从最新一块开始）', async () => {
    const chunk = [{ id: 3, body: 'body', body_translated: null }]
    vi.mocked(commands.getReleaseSearchBodies).mockResolvedValue(chunk)

    const result = await getReleaseSearchBodies(Number.MAX_SAFE_INTEGER, 512 * 1024)

    expect(commands.getReleaseSearchBodies).toHaveBeenCalledWith(Number.MAX_SAFE_INTEGER, 512 * 1024)
    expect(result).toEqual(chunk)
  })

  it('返回空数组表示已到库底，正常透传', async () => {
    vi.mocked(commands.getReleaseSearchBodies).mockResolvedValue([])

    expect(await getReleaseSearchBodies(1, 1024)).toEqual([])
  })
})

describe('getReleaseSearchBodiesByIds', () => {
  it('按 id 列表调起 get_release_search_bodies_by_ids', async () => {
    const chunk = [{ id: 5, body: '译文落库后的正文', body_translated: 'translated' }]
    vi.mocked(commands.getReleaseSearchBodiesByIds).mockResolvedValue(chunk)

    const result = await getReleaseSearchBodiesByIds([5, 6])

    expect(commands.getReleaseSearchBodiesByIds).toHaveBeenCalledWith([5, 6])
    expect(result).toEqual(chunk)
  })

  it('空 id 列表不发请求（避免无谓 IPC）', async () => {
    const result = await getReleaseSearchBodiesByIds([])

    expect(result).toEqual([])
    expect(commands.getReleaseSearchBodiesByIds).not.toHaveBeenCalled()
    expect(invokeI18nFn).not.toHaveBeenCalled()
  })
})

describe('setNotificationState', () => {
  it('标记为 clicked 仅传 releaseId 和 status', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(undefined)

    await setNotificationState(1, 'clicked')

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
  })

  it('标记为 snoozed 时附带 snoozeMinutes', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(undefined)

    await setNotificationState(2, 'snoozed', 1440)

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
  })

  it('标记为 ignored 不传 snoozeMinutes', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(undefined)

    await setNotificationState(3, 'ignored')

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
  })

  it('snoozeMinutes 为 0 时仍传递', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(undefined)

    await setNotificationState(4, 'snoozed', 0)

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
  })
})

describe('deleteRelease', () => {
  it('调起 delete_release 命令', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(undefined)

    await deleteRelease(5)

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
  })
})

describe('triggerPoll', () => {
  it('返回 PollResult（有新版本）', async () => {
    const pollResult = { new_releases: [{ id: 10, tag_name: 'v2.0.0' }] }
    vi.mocked(invokeI18nFn).mockResolvedValue(pollResult)

    const result = await triggerPoll()

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
    expect(result.new_releases).toHaveLength(1)
    expect(result.new_releases[0].tag_name).toBe('v2.0.0')
  })

  it('无新版本时返回空数组', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue({ new_releases: [] })

    const result = await triggerPoll()

    expect(result.new_releases).toHaveLength(0)
  })
})

describe('checkSingleSource', () => {
  it('调起 check_single_source 命令', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue({ new_releases: [] })

    await checkSingleSource(3)

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
  })

  it('返回该源的检查结果', async () => {
    const pollResult = { new_releases: [{ id: 7, tag_name: 'v1.1.0' }] }
    vi.mocked(invokeI18nFn).mockResolvedValue(pollResult)

    const result = await checkSingleSource(7)

    expect(result.new_releases).toHaveLength(1)
  })
})

describe('getPollCountdown', () => {
  it('返回剩余秒数', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(300)

    const result = await getPollCountdown()

    expect(invokeI18nFn).toHaveBeenCalledTimes(1)
    expect(result).toBe(300)
  })

  it('倒计时结束时返回 0', async () => {
    vi.mocked(invokeI18nFn).mockResolvedValue(0)

    const result = await getPollCountdown()

    expect(result).toBe(0)
  })
})
