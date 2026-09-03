// ── AgentWorkspace 会话上下文水位（H 域，评审 P1：上下文水位可见性）──
// 自 AgentWorkspace.vue 出仓：usage 状态 / loadUsage / 展示文案与告警判定。
// loadChat 的联动（预清 + 调用）经编排层把句柄传给聊天核心，本模块不反向依赖。
import { computed, ref, type Ref } from 'vue'
import { getAgentSessionUsage, type AgentSessionUsage } from '../../api/agent'
import { t } from '../../i18n'
import { formatCostUsd } from './agentChatUtils'

// 警告阈值（字符）：约 10 万 tokens 的中高水位（中文 token ≈ 字符数/2）。
// 模型上下文大小不一（128k~200k tokens），取保守中位，接近即提示开新会话。
const USAGE_WARN_CHARS = 200_000

export function useAgentUsage(activeKey: Ref<string>) {
  const usage = ref<AgentSessionUsage | null>(null)

  async function loadUsage() {
    try {
      usage.value = await getAgentSessionUsage(activeKey.value)
    } catch {
      usage.value = null
    }
  }

  const usageText = computed<string | null>(() => {
    const u = usage.value
    if (!u || u.message_count === 0) return null
    if (u.has_usage) {
      // pi 上报了真实用量：按计费口径展示（输入/输出分开，缓存命中不计入输入）
      const base = t(
        'agent.context_usage_actual',
        String(u.message_count),
        String(u.input_tokens),
        String(u.output_tokens),
      )
      // pi 未配置模型价格时 cost 全为 0（models.json 自定义模型默认单价 0），
      // 显示 $0.000000 会造成「免费」错觉——只展示词元，不拼成本段
      if (u.cost_micros === 0) return base
      return base + t('agent.cost_usage', formatCostUsd(u.cost_micros))
    }
    // 无上报数据：退回字符数估算（约 2 字符 / 词元）
    return t('agent.context_usage', String(u.message_count), String(Math.max(1, Math.round(u.total_chars / 2))))
  })

  const usageWarn = computed<boolean>(() => (usage.value?.total_chars ?? 0) > USAGE_WARN_CHARS)

  return { usage, loadUsage, usageText, usageWarn }
}
