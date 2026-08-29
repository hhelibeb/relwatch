import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type {
  AgentChatMessage,
  AgentConfig,
  AgentEntityRef,
  AgentModelRef,
  AgentModelsInfo,
  AgentQueueItem,
  AgentQueueStatus,
  AgentRpcStatus,
  AgentRunSummary,
  AgentSessionInfo,
  AgentSessionUsage,
} from '../bindings'

export type {
  AgentChatMessage,
  AgentConfig,
  AgentEntityRef,
  AgentModelRef,
  AgentModelsInfo,
  AgentQueueItem,
  AgentQueueStatus,
  AgentRpcStatus,
  AgentRunSummary,
  AgentSessionInfo,
  AgentSessionUsage,
  RpcAvailableModel,
} from '../bindings'

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
  /** 本次提交显式选择的模型（null = 跟随 pi 当前/默认模型）。 */
  model: AgentModelRef | null
  /** 本次提交附带的本地文件绝对路径（文件对话框自选；null/空 = 无附件）。 */
  files: string[] | null
}

/** 工作区提交：返回 run_id，终态经 AgentRunFinished 事件推送。 */
export async function runAgentJob(input: AgentJobInput): Promise<number> {
  return invokeI18nFn(() =>
    commands.runAgentJob({
      session_key: input.sessionKey,
      entities: input.entities,
      skill_path: input.skillPath,
      instruction: input.instruction,
      model: input.model,
      files: input.files,
    }),
  )
}

/** 查询 pi 当前可用模型（scope model）与当前激活模型（工作区模型下拉数据源）。 */
export async function getAgentAvailableModels(): Promise<AgentModelsInfo> {
  return invokeI18nFn(commands.getAgentAvailableModels)
}

/** 查询工作区会话的提交记录（倒序摘要，不含 stdout/stderr 大字段）。 */
export async function listAgentRuns(sessionKey: string, limit: number | null = 20): Promise<AgentRunSummary[]> {
  return invokeI18nFn(() => commands.listAgentRuns(sessionKey, limit))
}

/** 查询全局 Agent 队列状态（「排队中」提示：队列位置 + 其他会话占用）。 */
export async function getAgentQueueStatus(sessionKey: string): Promise<AgentQueueStatus> {
  return invokeI18nFn(() => commands.getAgentQueueStatus(sessionKey))
}

/** 查询全局队列（全部活跃 run，按执行顺序升序）：会话侧栏状态点 / 横幅「谁占用」数据源。 */
export async function getAgentQueue(): Promise<AgentQueueItem[]> {
  return invokeI18nFn(commands.getAgentQueue)
}

/** 查询会话文件的上下文水位（消息条数 / 文本字符数 / 文件字节数）。 */
export async function getAgentSessionUsage(sessionKey: string): Promise<AgentSessionUsage> {
  return invokeI18nFn(() => commands.getAgentSessionUsage(sessionKey))
}

/** 扫描磁盘上的会话文件，列出全部工作区会话（按最后活跃时间倒序）。
 *
 * 会话索引只存在于 localStorage（WebView2 缓存目录树，清缓存即失联），而会话文件
 * 在 Roaming 数据目录里完好无损。本接口即「磁盘发现」：前端把它与 localStorage
 * 索引合并，索引里没有的会话自动补入，落盘会话永不丢。
 */
export async function listAgentSessions(): Promise<AgentSessionInfo[]> {
  return invokeI18nFn(commands.listAgentSessions)
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

/** 查询 pi 常驻进程状态（健康指示 + 配置推迟提示）。
 *
 * 惰性：不会为了让指示灯变绿而主动拉起进程——进程未启动属正常（首次提交时才 spawn）。
 */
export async function getAgentRpcStatus(): Promise<AgentRpcStatus> {
  return invokeI18nFn(commands.getAgentRpcStatus)
}

/** 手动重启 pi 常驻进程。返回 false = 因有 run 正在执行而拒绝重启。 */
export async function restartAgentRpc(): Promise<boolean> {
  return invokeI18nFn(commands.restartAgentRpc)
}

/** 导出会话为 Markdown / JSON 文件（后端弹出保存对话框），返回实际写入路径。 */
export async function exportAgentSession(
  sessionKey: string,
  title: string,
  format: 'md' | 'json',
): Promise<string> {
  return invokeI18nFn(() => commands.exportAgentSession(sessionKey, title, format))
}
