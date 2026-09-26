import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ref } from 'vue'
import { t } from '../i18n'
import { useAgentUsage } from '../components/agent/useAgentUsage'
import { getAgentSessionUsage } from '../api/agent'

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return { ...actual, getAgentSessionUsage: vi.fn() }
})

/** 组装 AgentSessionUsage 测试样本（默认：无上下文水位、无用量）。 */
function makeUsage(over: Record<string, unknown> = {}) {
  return {
    message_count: 0,
    total_chars: 0,
    file_bytes: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    total_tokens: 0,
    cost_micros: 0,
    has_usage: false,
    context_tokens: null,
    context_window: null,
    context_estimated: false,
    auto_compaction: true,
    ...over,
  }
}

beforeEach(() => {
  vi.mocked(getAgentSessionUsage).mockReset()
})

describe('useAgentUsage', () => {
  it('loadUsage 成功写入，失败置 null', async () => {
    const activeKey = ref('k1')
    const { usage, loadUsage } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(makeUsage({ message_count: 3, total_chars: 4000 }))
    await loadUsage()
    expect(usage.value?.message_count).toBe(3)
    expect(getAgentSessionUsage).toHaveBeenCalledWith('k1')

    vi.mocked(getAgentSessionUsage).mockRejectedValue(new Error('boom'))
    await loadUsage()
    expect(usage.value).toBeNull()
  })

  it('usageText：水位打头（百分比 / 窗口），其后接累计词元与成本', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText } = useAgentUsage(activeKey)
    expect(usageText.value).toBeNull() // 无数据

    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({
        message_count: 136,
        total_chars: 4000,
        input_tokens: 240,
        output_tokens: 1243,
        cost_micros: 1674,
        has_usage: true,
        context_tokens: 115_915,
        context_window: 1_000_000,
      }),
    )
    await loadUsage()
    expect(usageText.value).toBe(
      [
        t('agent.context_waterline', '11.6', '1.0M') + ` ${t('agent.context_auto_label')}`,
        t('agent.context_usage_actual', '136', '240', '1243') + t('agent.cost_usage', '0.001674'),
      ].join(' · '),
    )
  })

  it('usageText：自动压缩关闭时水位不带 (auto)', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 4, has_usage: true, context_tokens: 64_000, context_window: 128_000, auto_compaction: false }),
    )
    await loadUsage()
    expect(usageText.value).toBe(
      [t('agent.context_waterline', '50.0', '128k'), t('agent.context_usage_actual', '4', '0', '0')].join(' · '),
    )
    expect(usageText.value).not.toContain(t('agent.context_auto_label'))
  })

  it('usageText：成本为零时不拼 $ 段', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({
        message_count: 4,
        input_tokens: 900,
        output_tokens: 120,
        cost_micros: 0,
        has_usage: true,
        context_tokens: 12_000,
        context_window: 200_000,
      }),
    )
    await loadUsage()
    expect(usageText.value).toBe(
      [
        t('agent.context_waterline', '6.0', '200k') + ` ${t('agent.context_auto_label')}`,
        t('agent.context_usage_actual', '4', '900', '120'),
      ].join(' · '),
    )
    expect(usageText.value).not.toContain('$')
  })

  it('usageText：压缩后水位未知 → `? / 窗口`（与 pi footer 同表现）', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText, usageHint } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 30, total_chars: 4000, context_tokens: null, context_window: 1_000_000 }),
    )
    await loadUsage()
    expect(usageText.value).toBe(
      [t('agent.context_unknown', '1.0M') + ` ${t('agent.context_auto_label')}`, t('agent.context_usage', '30', '2000')].join(' · '),
    )
    expect(usageHint.value).toBe(t('agent.context_unknown_hint'))
  })

  it('usageText：窗口查不到时不显示百分比（宁可不显示也不猜分母）', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 6, total_chars: 4000, input_tokens: 1200, output_tokens: 340, has_usage: true, context_tokens: 50_000 }),
    )
    await loadUsage()
    expect(usageText.value).toBe(t('agent.context_usage_actual', '6', '1200', '340'))
    expect(usageText.value).not.toContain('%')
  })

  it('usageText：无上报数据回落字符数估算（约 2 字符/词元，下限 1）', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(makeUsage({ message_count: 3, total_chars: 4000 }))
    await loadUsage()
    expect(usageText.value).toBe(t('agent.context_usage', '3', '2000'))

    vi.mocked(getAgentSessionUsage).mockResolvedValue(makeUsage({ message_count: 2, total_chars: 1 }))
    await loadUsage()
    expect(usageText.value).toBe(t('agent.context_usage', '2', '1'))
  })

  it('usageText：message_count 为 0 时不展示', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageText } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(makeUsage({ message_count: 0, total_chars: 999, has_usage: true }))
    await loadUsage()
    expect(usageText.value).toBeNull()
  })

  it('usageEstimated：水位或累计任一为估算就标 ≈', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageEstimated } = useAgentUsage(activeKey)
    // pi 未上报用量（水位也是 chars/4 估算）
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 3, total_chars: 4000, context_tokens: 2000, context_window: 1_000_000, context_estimated: true }),
    )
    await loadUsage()
    expect(usageEstimated.value).toBe(true)

    // pi 上报了真实用量 → 不标
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 3, has_usage: true, context_tokens: 2000, context_window: 1_000_000 }),
    )
    await loadUsage()
    expect(usageEstimated.value).toBe(false)
  })

  it('usageHint：自动压缩开启时提示 (auto) 的含义', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageHint } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 3, has_usage: true, context_tokens: 2000, context_window: 1_000_000 }),
    )
    await loadUsage()
    expect(usageHint.value).toBe(t('agent.context_auto_hint'))

    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 3, has_usage: true, context_tokens: 2000, context_window: 1_000_000, auto_compaction: false }),
    )
    await loadUsage()
    expect(usageHint.value).toBeUndefined()
  })
})
