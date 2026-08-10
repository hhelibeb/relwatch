import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

// 模块级单例：每个用例重新加载模块以获得干净的 pending/enabled 状态
async function loadTracking() {
  vi.resetModules()
  return await import('../composables/useUsageTracking')
}

describe('useUsageTracking', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.clearAllMocks()
  })

  it('track 累积计数，5s 后批量写入后端', async () => {
    const { track } = await loadTracking()
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    track('source.add')
    track('source.add')
    track('source.remove')
    // 5s 内不应写入
    expect(invoke).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(5000)
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith('record_usage', {
      events: [
        ['source.add', 2],
        ['source.remove', 1],
      ],
    })
  })

  it('重复 track 共享同一批写入（定时器只挂一个）', async () => {
    const { track } = await loadTracking()
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    track('a')
    await vi.advanceTimersByTimeAsync(2500)
    track('b') // 定时器已挂，不重复挂
    await vi.advanceTimersByTimeAsync(2500)

    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith('record_usage', {
      events: [
        ['a', 1],
        ['b', 1],
      ],
    })
  })

  it('flushUsageTrackingNow 立即冲刷未到期的计数', async () => {
    const { track, flushUsageTrackingNow } = await loadTracking()
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    track('source.check')
    flushUsageTrackingNow()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith('record_usage', {
      events: [['source.check', 1]],
    })
    // 冲刷后再次 flush 不应重复写入
    flushUsageTrackingNow()
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('写入失败静默（不影响功能，不抛错）', async () => {
    const { track, flushUsageTrackingNow } = await loadTracking()
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('boom'))

    track('source.add')
    await expect(flushUsageTrackingNow()).resolves.toBeUndefined()
  })

  it('setUsageTrackingEnabled(false) 后 track no-op 且丢弃未上报计数', async () => {
    const { track, flushUsageTrackingNow, setUsageTrackingEnabled } = await loadTracking()
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    track('source.add')
    setUsageTrackingEnabled(false)
    track('source.remove') // 关闭后不应记录
    flushUsageTrackingNow()

    // 关闭时清空了 pending，且关闭后 track 不产生计数 → 无写入
    expect(invoke).not.toHaveBeenCalled()
  })

  it('空 key 忽略', async () => {
    const { track, flushUsageTrackingNow } = await loadTracking()
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    track('')
    flushUsageTrackingNow()
    expect(invoke).not.toHaveBeenCalled()
  })
})
