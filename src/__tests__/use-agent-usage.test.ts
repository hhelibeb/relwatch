import { describe, it, expect, vi, beforeEach } from 'vitest'
import { ref } from 'vue'
import { t } from '../i18n'
import { useAgentUsage } from '../components/agent/useAgentUsage'
import { getAgentSessionUsage } from '../api/agent'

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return { ...actual, getAgentSessionUsage: vi.fn() }
})

/** 组装 AgentSessionUsage 测试样本。 */
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

  it('usageText：has_usage + 有成本 → 词元 + 成本；成本为零不拼 $ 段', async () => {
    const activeKey = ref('k')
    const { usage, loadUsage, usageText } = useAgentUsage(activeKey)
    expect(usageText.value).toBeNull() // 无数据

    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 6, total_chars: 4000, input_tokens: 1200, output_tokens: 340, cost_micros: 41244, has_usage: true }),
    )
    await loadUsage()
    expect(usageText.value).toBe(
      t('agent.context_usage_actual', '6', '1200', '340') + t('agent.cost_usage', '0.0412'),
    )

    vi.mocked(getAgentSessionUsage).mockResolvedValue(
      makeUsage({ message_count: 4, total_chars: 3000, input_tokens: 900, output_tokens: 120, cost_micros: 0, has_usage: true }),
    )
    await loadUsage()
    expect(usageText.value).toBe(t('agent.context_usage_actual', '4', '900', '120'))
    expect(usageText.value).not.toContain('$')
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

  it('usageWarn：超过 20 万字符告警', async () => {
    const activeKey = ref('k')
    const { loadUsage, usageWarn } = useAgentUsage(activeKey)
    vi.mocked(getAgentSessionUsage).mockResolvedValue(makeUsage({ message_count: 1, total_chars: 200_000 }))
    await loadUsage()
    expect(usageWarn.value).toBe(false) // 恰在阈值不告警

    vi.mocked(getAgentSessionUsage).mockResolvedValue(makeUsage({ message_count: 1, total_chars: 200_001 }))
    await loadUsage()
    expect(usageWarn.value).toBe(true)
  })
})
