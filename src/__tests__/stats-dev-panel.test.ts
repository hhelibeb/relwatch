import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import StatsDevPanel from '../dev/StatsDevPanel.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ confirm: vi.fn() }))

const MOCK_ROWS = [
  {
    key: 'source.add',
    total_count: 12,
    last_day: '2026-02-01',
    daily: [{ day: '2026-02-01', count: 12 }],
  },
  {
    key: 'release.translate',
    total_count: 5,
    last_day: '2026-02-02',
    daily: [
      { day: '2026-02-01', count: 2 },
      { day: '2026-02-02', count: 3 },
    ],
  },
]

afterEach(() => {
  vi.clearAllMocks()
})

/** 通用 mock：get_usage_stats 按调用次数依次返回 provider 提供的结果。 */
async function mockInvoke(getResults: unknown[][]) {
  const { invoke } = await import('@tauri-apps/api/core')
  let getCalls = 0
  ;(invoke as ReturnType<typeof vi.fn>).mockImplementation((cmd: string) => {
    if (cmd === 'get_usage_stats') {
      const idx = Math.min(getCalls, getResults.length - 1)
      getCalls++
      return Promise.resolve(getResults[idx])
    }
    return Promise.resolve(undefined)
  })
  return { invoke }
}

describe('StatsDevPanel', () => {
  it('渲染排行榜：事件名 + 次数 + 最近点击', async () => {
    await mockInvoke([MOCK_ROWS])
    const wrapper = mount(StatsDevPanel)
    await flushPromises()

    expect(wrapper.text()).toContain('功能使用统计')
    expect(wrapper.text()).toContain('添加源')
    expect(wrapper.text()).toContain('12')
    expect(wrapper.text()).toContain('AI 翻译')
    expect(wrapper.text()).toContain('5')
    expect(wrapper.text()).toContain('17') // 总点击
  })

  it('无映射的事件 key 兜底显示原始 key', async () => {
    await mockInvoke([[{ key: 'custom.future_event', total_count: 1, last_day: '2026-02-01', daily: [] }]])
    const wrapper = mount(StatsDevPanel)
    await flushPromises()
    expect(wrapper.text()).toContain('custom.future_event')
  })

  it('空数据显示空状态', async () => {
    await mockInvoke([[]])
    const wrapper = mount(StatsDevPanel)
    await flushPromises()
    expect(wrapper.text()).toContain('暂无统计数据')
  })

  it('清空按钮二次确认后调用 clear_usage_stats 并刷新', async () => {
    await mockInvoke([MOCK_ROWS, []])
    const { confirm } = await import('@tauri-apps/plugin-dialog')
    ;(confirm as ReturnType<typeof vi.fn>).mockResolvedValue(true)
    const { invoke } = await import('@tauri-apps/api/core')

    const wrapper = mount(StatsDevPanel)
    await flushPromises()
    expect(wrapper.text()).toContain('添加源')

    await wrapper.get('.btn-danger').trigger('click')
    await flushPromises()

    expect(confirm).toHaveBeenCalled()
    expect(invoke).toHaveBeenCalledWith('clear_usage_stats')
    expect(wrapper.text()).toContain('暂无统计数据')
  })

  it('点击遮罩触发 close 事件', async () => {
    await mockInvoke([MOCK_ROWS])
    const wrapper = mount(StatsDevPanel)
    await flushPromises()
    await wrapper.get('.stats-dev-overlay').trigger('click')
    expect(wrapper.emitted('close')).toBeTruthy()
  })
})
