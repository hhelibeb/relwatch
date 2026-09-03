// ── AgentWorkspace 聊天区纯函数（模块级，无组件状态）──
// 自 AgentWorkspace.vue 出仓：消息块拆分 / 工具卡片文案 / run 记录解析。
// 依赖 i18n 文案的函数（runErrorText / runModelLabel / runDurationText）以 `t`
// 为参注入（调用点传入），不直接 import i18n 实例——保持模块纯函数性质与单测便利。
import type { AgentChatBlock, AgentChatMessage, AgentModelRef, AgentRunSummary } from '../../bindings'
import type { AgentEntityRefSeed } from '../../injection-keys'
import { InvokeI18nError } from '../../api/client'

/** i18n 翻译函数形状（与 src/i18n 的 t 一致，注入以解耦全局 locale 状态）。 */
export type TranslateFn = (key: string, ...args: string[]) => string

/** 正则转义：skill 短名可能含 . - 等元字符（如 code-review）。 */
export function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** pi 展开 /skill: 命令时把 skill 全文注入 user 消息（<skill name=…>…</skill>）。
 * 折叠为空白：skill 徽章由 run.skill_path 渲染，避免全文刷屏（对齐 pi TUI 的「加载 Skill」提示）。 */
export function stripSkillBlock(text: string): string {
  return text.replace(/<skill name="[^"]*"[^>]*>[\s\S]*?<\/skill>\s*/, '')
}

/** 用户气泡显示文本拆分（纯函数）：
 * - main：<用户指令> 标签内的用户真实指令（skill 块已剥离）
 * - folded：标签外的模板脚手架（订阅说明 / 外部数据区 / 不可信声明等）
 * 首轮完整模板不再整段刷屏，折叠为可展开的详情块，完整上下文仍可见；
 * 无标签（旧格式 / 多轮精简）时整段作为主文本，行为不变。 */
export function splitUserBlocks(blocks: AgentChatBlock[]): { main: string; folded: string | null } {
  const text = blocks
    .filter((b) => b.kind === 'text')
    .map((b) => (b as { kind: 'text'; text?: string }).text ?? '')
    .join('\n')
  const cleaned = stripSkillBlock(text)
  const m = cleaned.match(/<用户指令>\s*([\s\S]*?)\s*<\/用户指令>/)
  if (!m) return { main: cleaned.trim(), folded: null }
  const folded = cleaned.replace(m[0], '').trim()
  return { main: m[1].trim(), folded: folded || null }
}

export function isToolError(msg: AgentChatMessage): boolean {
  return msg.blocks.some((b) => b.kind === 'toolResult' && b.is_error)
}

export function toolCardName(msg: AgentChatMessage): string {
  for (const b of msg.blocks) {
    if (b.kind === 'toolResult') return b.tool_name
    if (b.kind === 'bash') return 'bash'
  }
  return msg.role
}

export function bashExitLabel(msg: AgentChatMessage): string {
  const b = msg.blocks.find((x) => x.kind === 'bash') as Extract<AgentChatBlock, { kind: 'bash' }> | undefined
  return b ? `exit ${b.exit_code ?? '?'}` : ''
}

export function toolCardBody(msg: AgentChatMessage): string {
  const parts: string[] = []
  for (const b of msg.blocks) {
    if (b.kind === 'toolResult') {
      parts.push(b.text)
    } else if (b.kind === 'bash') {
      parts.push(`$ ${b.command}`, b.output)
    }
  }
  return parts.filter((p) => p.trim()).join('\n')
}

export function runEntities(run: AgentRunSummary | undefined): AgentEntityRefSeed[] {
  if (!run) return []
  try {
    return JSON.parse(run.entities) as AgentEntityRefSeed[]
  } catch {
    return []
  }
}

/** run 记录固化的模型选择（JSON 字符串；解析失败按「默认」处理）。 */
export function runModel(run: AgentRunSummary): AgentModelRef | null {
  if (!run.model) return null
  try {
    return JSON.parse(run.model) as AgentModelRef
  } catch {
    return null
  }
}

export function runModelLabel(run: AgentRunSummary, t: TranslateFn): string {
  const m = runModel(run)
  return m ? m.model_id : t('agent.run_model_default')
}

/** run 记录固化的本地文件附件（JSON 字符串数组；解析失败按无附件处理）。 */
export function runFiles(run: AgentRunSummary): string[] {
  if (!run.files) return []
  try {
    const parsed = JSON.parse(run.files) as unknown
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string') : []
  } catch {
    return []
  }
}

/** 失败原因文案：run.error 形如 `err.agent.timeout|300`（i18n 键|参数）。 */
export function runErrorText(run: AgentRunSummary | undefined, t: TranslateFn): string | null {
  if (!run?.error) return null
  const [key, ...args] = run.error.split('|')
  const text = t(key, ...args)
  // i18n 未命中时 t() 原样返回 key：不渲染裸键
  return text === key ? null : text
}

/** 该 run 是否值得重试（非成功终态即终点不明：失败 / 超时 / 被取消 / 结果未知）。 */
export function canRetry(run: AgentRunSummary | undefined): boolean {
  if (!run) return false
  return run.status === 'failed' || run.status === 'timeout' || run.status === 'cancelled' || run.status === 'unknown'
}

export function runDurationText(run: AgentRunSummary, t: TranslateFn): string {
  if (!run.started_at || !run.finished_at) return '—'
  const start = new Date(run.started_at).getTime()
  const end = new Date(run.finished_at).getTime()
  if (!Number.isFinite(start) || !Number.isFinite(end)) return '—'
  const secs = Math.max(0, Math.round((end - start) / 1000))
  // 耗时文案走 i18n（英文界面不再漏中文）
  if (secs < 60) return t('agent.duration_secs', String(secs))
  const mins = Math.floor(secs / 60)
  if (mins < 60) return secs % 60 > 0 ? t('agent.duration_min_secs', String(mins), String(secs % 60)) : t('agent.duration_min', String(mins))
  return t('agent.duration_hour_min', String(Math.floor(mins / 60)), String(mins % 60))
}

/** 两个 RFC3339 时间是否在 60 秒窗内（run_id 直连与时间窗兜底的共同校验）。 */
export function timeAdjacent(a: string, b: string): boolean {
  const ta = new Date(a).getTime()
  const tb = new Date(b).getTime()
  return Number.isFinite(ta) && Number.isFinite(tb) && Math.abs(ta - tb) < 60_000
}

/** 成本微元 → 美元展示串。
 *  LLM 单次成本常在 1e-5 ~ 1e-1 美元区间，固定 2 位小数会全显示成 $0.00（信息量为零），
 *  固定 6 位又会让大额变得难读；故按量级自适应保留有效小数位。 */
export function formatCostUsd(micros: number): string {
  const usd = micros / 1e6
  if (usd >= 1) return usd.toFixed(2)
  if (usd >= 0.01) return usd.toFixed(4)
  return usd.toFixed(6)
}

/** 取错误的 i18n key（`err.*`），用于按错误类型分支而非比对翻译后的文案。
 *  InvokeI18nError 直接带 key；其余情况仅当原文就是未翻译的 err.* 键时返回。 */
export function errorKey(e: unknown): string | null {
  if (e instanceof InvokeI18nError) return e.key
  const raw = (e instanceof Error ? e.message : String(e)).replace(/^Error:\s*/, '')
  return raw.startsWith('err.') ? raw.split('|')[0] : null
}
