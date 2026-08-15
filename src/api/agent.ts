import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { AgentChatMessage, AgentConfig, AgentEntityRef, AgentRun } from '../bindings'

export type { AgentChatMessage, AgentConfig, AgentEntityRef, AgentRun } from '../bindings'

/** 读取全局 Agent 配置。 */
export async function getAgentConfig(): Promise<AgentConfig> {
  return invokeI18nFn(commands.getAgentConfig)
}

/** 保存全局 Agent 配置（skill 列表由后端去重/trim）。 */
export async function saveAgentConfig(config: AgentConfig): Promise<void> {
  await invokeI18nFn(() => commands.saveAgentConfig(config))
}

/** 工作区提交入参（camelCase，内部转 snake_case 传后端）。 */
export interface AgentJobInput {
  sessionKey: string
  entities: AgentEntityRef[]
  skillPath: string | null
  instruction: string
}

/** 工作区提交：返回 run_id，终态经 AgentRunFinished 事件推送。 */
export async function runAgentJob(input: AgentJobInput): Promise<number> {
  return invokeI18nFn(() =>
    commands.runAgentJob({
      session_key: input.sessionKey,
      entities: input.entities,
      skill_path: input.skillPath,
      instruction: input.instruction,
    }),
  )
}

/** 查询工作区会话的提交记录（倒序）。 */
export async function listAgentRuns(sessionKey: string, limit: number | null = 20): Promise<AgentRun[]> {
  return invokeI18nFn(() => commands.listAgentRuns(sessionKey, limit))
}

/** 读取会话的完整聊天消息流（pi 落盘 JSONL，时间正序）。 */
export async function listAgentMessages(sessionKey: string): Promise<AgentChatMessage[]> {
  return invokeI18nFn(() => commands.listAgentMessages(sessionKey))
}

/** 取消一次正在运行（或排队中）的 Agent 提交。 */
export async function cancelAgentRun(runId: number): Promise<void> {
  await invokeI18nFn(() => commands.cancelAgentRun(runId))
}

/** 删除一个工作区会话（会话文件 + 全部运行记录）。 */
export async function deleteAgentSession(sessionKey: string): Promise<void> {
  await invokeI18nFn(() => commands.deleteAgentSession(sessionKey))
}

/** 在终端恢复该次会话的命令字符串（供复制）。 */
export async function getAgentSessionCommand(runId: number): Promise<string> {
  return invokeI18nFn(() => commands.getAgentSessionCommand(runId))
}

/** 在新终端窗口中打开该次运行的 pi 会话（恢复完整执行过程）。 */
export async function openAgentSession(runId: number): Promise<void> {
  await invokeI18nFn(() => commands.openAgentSession(runId))
}
