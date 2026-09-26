// ── AgentWorkspace 会话上下文水位（H 域）──
// 自 AgentWorkspace.vue 出仓：usage 状态 / loadUsage / 展示文案。
// loadChat 的联动（预清 + 调用）经编排层把句柄传给聊天核心，本模块不反向依赖。
//
// 词元与窗口口径对齐 pi footer 的 `5.2% / 1.0M (auto)`：数字全部由后端
//（agent_context.rs，复刻 pi 的 getContextUsage）算好，前端只负责展示与百分比。
// 早先「整个会话字符数 ÷ 2」的估算与 pi 实测偏差可达 1.7 倍（8.2% vs 4.7%），
// 已由 `context_tokens / context_window` 取代。
import { computed, ref, type Ref } from 'vue'
import { getAgentSessionUsage, type AgentSessionUsage } from '../../api/agent'
import { t } from '../../i18n'
import { formatCostUsd, formatTokenCount } from './agentChatUtils'

export function useAgentUsage(activeKey: Ref<string>) {
  const usage = ref<AgentSessionUsage | null>(null)

  async function loadUsage() {
    try {
      usage.value = await getAgentSessionUsage(activeKey.value)
    } catch {
      usage.value = null
    }
  }

  /** 词元与成本累计行（回答「花了多少」，与上下文水位口径不同）。 */
  const totalsText = computed<string>(() => {
    const u = usage.value
    if (!u) return ''
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

  /** 上下文水位段：`11.6% / 1.0M (auto)`；未知水位 → `? / 1.0M`；窗口未知 → ''（不显示）。 */
  const waterlineText = computed<string>(() => {
    const u = usage.value
    if (!u || u.message_count === 0) return ''
    const window = u.context_window
    if (window === null || window <= 0) {
      // 窗口查不到（模型不在 pi 模型目录里）时百分比无从谈起：宁可不显示，
      // 也不猜一个默认窗口——错误的分母比没有数字更误导
      return ''
    }
    const auto = u.auto_compaction ? ` ${t('agent.context_auto_label')}` : ''
    const windowLabel = formatTokenCount(window)
    if (u.context_tokens === null) {
      // 压缩后还没有新一轮响应：旧 usage 反映的是压缩前的上下文，pi 同样显示 `?`
      return t('agent.context_unknown', windowLabel) + auto
    }
    return t(
      'agent.context_waterline',
      ((u.context_tokens / window) * 100).toFixed(1),
      windowLabel,
    ) + auto
  })

  const usageText = computed<string | null>(() => {
    const u = usage.value
    if (!u || u.message_count === 0) return null
    return [waterlineText.value, totalsText.value].filter(Boolean).join(' · ')
  })

  /** 数字是否为估算（前端据此标 ≈，不把估算值当精确值展示）。 */
  const usageEstimated = computed<boolean>(
    () => !!usage.value && (usage.value.context_estimated || !usage.value.has_usage),
  )

  /** 水位悬浮说明：解释 `?`（压缩后未知）与 `(auto)`（自动压缩已开）两种非直观状态。 */
  const usageHint = computed<string | undefined>(() => {
    const u = usage.value
    if (!u || u.message_count === 0) return undefined
    if (u.context_window !== null && u.context_tokens === null) {
      return t('agent.context_unknown_hint')
    }
    if (u.auto_compaction) return t('agent.context_auto_hint')
    return undefined
  })

  return { usage, loadUsage, usageText, usageEstimated, usageHint }
}
