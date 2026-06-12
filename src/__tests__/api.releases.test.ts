import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../api/client', () => ({
  invokeI18n: vi.fn(),
  openReleaseUrl: vi.fn(),
  translateError: vi.fn((raw: string) => raw),
}))

import { invokeI18n } from '../api/client'
import {
  getReleases,
  setNotificationState,
  deleteRelease,
  triggerPoll,
  checkSingleSource,
  getPollCountdown,
} from '../api/releases'

beforeEach(() => {
  vi.clearAllMocks()
})

describe('getReleases', () => {
  it('调起 get_releases 命令，返回 ReleaseInfo[]', async () => {
    const mockData = [{ id: 1, tag_name: 'v1.0.0', owner: 'test', repo: 'test' }]
    vi.mocked(invokeI18n).mockResolvedValue(mockData)

    const result = await getReleases()

    expect(invokeI18n).toHaveBeenCalledWith('get_releases')
    expect(result).toEqual(mockData)
  })

  it('后端返回空数组时正常透传', async () => {
    vi.mocked(invokeI18n).mockResolvedValue([])

    const result = await getReleases()

    expect(result).toEqual([])
  })

  it('后端错误时抛出异常', async () => {
    vi.mocked(invokeI18n).mockRejectedValue(new Error('err.database'))

    await expect(getReleases()).rejects.toThrow('err.database')
  })
})

describe('setNotificationState', () => {
  it('标记为 clicked 仅传 releaseId 和 status', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setNotificationState(1, 'clicked')

    expect(invokeI18n).toHaveBeenCalledWith('set_notification_state', {
      releaseId: 1,
      status: 'clicked',
    })
  })

  it('标记为 snoozed 时附带 snoozeMinutes', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setNotificationState(2, 'snoozed', 1440)

    expect(invokeI18n).toHaveBeenCalledWith('set_notification_state', {
      releaseId: 2,
      status: 'snoozed',
      snoozeMinutes: 1440,
    })
  })

  it('标记为 ignored 不传 snoozeMinutes', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setNotificationState(3, 'ignored')

    expect(invokeI18n).toHaveBeenCalledWith('set_notification_state', {
      releaseId: 3,
      status: 'ignored',
    })
  })

  it('snoozeMinutes 为 0 时仍传递', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await setNotificationState(4, 'snoozed', 0)

    expect(invokeI18n).toHaveBeenCalledWith('set_notification_state', {
      releaseId: 4,
      status: 'snoozed',
      snoozeMinutes: 0,
    })
  })
})

describe('deleteRelease', () => {
  it('调起 delete_release 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(undefined)

    await deleteRelease(5)

    expect(invokeI18n).toHaveBeenCalledWith('delete_release', { releaseId: 5 })
  })
})

describe('triggerPoll', () => {
  it('返回 PollResult（有新版本）', async () => {
    const pollResult = { new_releases: [{ id: 10, tag_name: 'v2.0.0' }] }
    vi.mocked(invokeI18n).mockResolvedValue(pollResult)

    const result = await triggerPoll()

    expect(invokeI18n).toHaveBeenCalledWith('trigger_poll')
    expect(result.new_releases).toHaveLength(1)
    expect(result.new_releases[0].tag_name).toBe('v2.0.0')
  })

  it('无新版本时返回空数组', async () => {
    vi.mocked(invokeI18n).mockResolvedValue({ new_releases: [] })

    const result = await triggerPoll()

    expect(result.new_releases).toHaveLength(0)
  })
})

describe('checkSingleSource', () => {
  it('调起 check_single_source 命令', async () => {
    vi.mocked(invokeI18n).mockResolvedValue({ new_releases: [] })

    await checkSingleSource(3)

    expect(invokeI18n).toHaveBeenCalledWith('check_single_source', { id: 3 })
  })

  it('返回该源的检查结果', async () => {
    const pollResult = { new_releases: [{ id: 7, tag_name: 'v1.1.0' }] }
    vi.mocked(invokeI18n).mockResolvedValue(pollResult)

    const result = await checkSingleSource(7)

    expect(result.new_releases).toHaveLength(1)
  })
})

describe('getPollCountdown', () => {
  it('返回剩余秒数', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(300)

    const result = await getPollCountdown()

    expect(invokeI18n).toHaveBeenCalledWith('get_poll_countdown')
    expect(result).toBe(300)
  })

  it('倒计时结束时返回 0', async () => {
    vi.mocked(invokeI18n).mockResolvedValue(0)

    const result = await getPollCountdown()

    expect(result).toBe(0)
  })
})
