<script setup lang="ts">
import { ref, computed, inject, onMounted, onUnmounted, nextTick, watch } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { confirm } from '@tauri-apps/plugin-dialog'
import MarkdownContent from './common/MarkdownContent.vue'
import { events, type AgentRunFinished } from '../bindings'
import { ShowToastKey, type AgentEntityRefSeed, type AgentWorkspaceSeed } from '../injection-keys'
import {
  getAgentConfig,
  getAgentAvailableModels,
  runAgentJob,
  saveAgentConfig,
  listAgentRuns,
  listAgentMessages,
  listAgentSessions,
  deleteAgentSession,
  cancelAgentRun,
  openAgentSession,
  getAgentSessionCommand,
  getAgentQueueStatus,
  getAgentQueue,
  getAgentSessionUsage,
  type AgentChatMessage,
  type AgentModelRef,
  type AgentQueueItem,
  type AgentQueueStatus,
  type AgentRunSummary,
  type AgentSessionInfo,
  type AgentSessionUsage,
  type RpcAvailableModel,
} from '../api/agent'
import { listSources, type Source } from '../api/sources'
import { getReleases, type ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import { formatDate, skillShortName } from '../utils'
import { track } from '../composables/useUsageTracking'

const props = defineProps<{ seed?: AgentWorkspaceSeed | null; width?: number }>()
const emit = defineEmits<{ close: [] }>()
const showToast = inject(ShowToastKey, () => {})
// 面板宽度：默认 440（CSS 兜底同值），App.vue 展开时传入持久化宽度
const panelWidth = computed(() => props.width ?? 440)

// ── 会话元信息（localStorage 持久化，窗口重开可继续对话）──────────
interface SessionMeta {
  key: string
  title: string
  updatedAt: number
  /** 该会话显式选择的模型（null/缺省 = 跟随 pi 当前/默认模型）。 */
  model?: AgentModelRef | null
  /** 由磁盘文件发现补入（localStorage 索引里没有）：侧栏标记为「已恢复」。 */
  recovered?: boolean
  /** 未提交的草稿会话（新建即登记；提交成功后清除）。 */
  draft?: boolean
}
const SESSIONS_STORAGE_KEY = 'relwatch.agent.sessions.v1'
// 会话侧栏折叠状态（默认折叠，聊天区全宽；localStorage 持久化）
const SIDEBAR_STORAGE_KEY = 'relwatch.agent.sidebar.v1'
// 会话 meta 持久化上限：每条约 150 字节，200 条仅 ~30KB，
// 远低于 localStorage 配额；超出部分由「清理旧会话」入口回收磁盘文件与 DB 记录
const SESSIONS_META_LIMIT = 200
const sidebarOpen = ref(localStorage.getItem(SIDEBAR_STORAGE_KEY) === '1')

function toggleSidebar() {
  sidebarOpen.value = !sidebarOpen.value
  localStorage.setItem(SIDEBAR_STORAGE_KEY, sidebarOpen.value ? '1' : '0')
}

function loadSessions(): SessionMeta[] {
  try {
    const raw = localStorage.getItem(SESSIONS_STORAGE_KEY)
    const parsed = raw ? (JSON.parse(raw) as SessionMeta[]) : []
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}
function persistSessions() {
  localStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify(sessions.value.slice(0, SESSIONS_META_LIMIT)))
}

/** 磁盘发现：会话索引只存在于 localStorage（WebView2 缓存目录树，清缓存即失联），
 * 而会话文件在 Roaming 数据目录里完好无损 —— 文件即索引，标题从首条 user 消息重建。
 *
 * 合并策略：localStorage 为准（用户改过的标题/模型优先），磁盘上有而索引中没有的
 * 会话自动补入并标记为「恢复的会话」。用户点开后标记清除（已确认，不再是异常态）。 */
async function discoverSessions(): Promise<number> {
  let found: AgentSessionInfo[]
  try {
    found = await listAgentSessions()
  } catch {
    return 0 // 发现失败不阻塞（localStorage 索引仍可用）
  }
  const known = new Set(sessions.value.map((s) => s.key))
  const recovered: SessionMeta[] = []
  for (const s of found) {
    if (known.has(s.session_key)) continue
    recovered.push({
      key: s.session_key,
      title: s.title.trim() || t('agent.session_untitled'),
      updatedAt: new Date(s.updated_at).getTime() || Date.now(),
      recovered: true,
    })
  }
  if (recovered.length === 0) return 0
  sessions.value = [...sessions.value, ...recovered].sort((a, b) => b.updatedAt - a.updatedAt)
  persistSessions()
  return recovered.length
}

const sessions = ref<SessionMeta[]>(loadSessions())
// 「新建即登记」：无历史会话时立即登记一个草稿会话（标题「新会话」）——
// 任何时刻 activeKey 都对应索引中的一项，未提交的会话不因重启/关面板丢失。
// （此前「点新会话→拖实体→写半句话→关闭」的 key 永久丢失，见评审 1.2）
if (sessions.value.length === 0) {
  sessions.value = [{ key: newSessionKey(), title: t('agent.session_new'), updatedAt: Date.now(), draft: true }]
}
persistSessions()
// 激活会话：最近一个优先
const activeKey = ref(sessions.value[0].key)
const sessionTitle = computed(() => {
  const meta = sessions.value.find((s) => s.key === activeKey.value)
  return meta ? meta.title : t('agent.session_new')
})

function newSessionKey(): string {
  return crypto.randomUUID()
}

/** 清除会话的「已恢复」标记（用户打开过即视为已确认），变更时写回索引。 */
function clearRecoveredFlag(key: string) {
  const idx = sessions.value.findIndex((s) => s.key === key && s.recovered)
  if (idx < 0) return
  sessions.value[idx] = { ...sessions.value[idx], recovered: false }
  persistSessions()
}

function switchSession(key: string) {
  if (key === activeKey.value) return
  // 不中止原会话的 run：后端并发上限 1、其余排队执行（pending 取消只插标记不碰进程），
  // 切回会话时由 loadChat 从 runs 推导恢复停止按钮——各会话独立启停，互不误杀
  activeKey.value = key
  stopPolling()
  submittedRunId.value = null
  cancelling.value = false
  liveMessages.value = []
  historySnapshot.value = []
  // 恢复的会话被打开过即转为普通会话（已确认，不再是异常态）；同时写回索引
  clearRecoveredFlag(key)
  selectedModel.value = sessions.value.find((s) => s.key === key)?.model ?? null
  void loadChat()
  entities.value = []
  skillPath.value = null
  instruction.value = ''
}

function startNewSession() {
  // 当前已是未提交草稿且无内容 → 不重复新建
  const cur = sessions.value.find((s) => s.key === activeKey.value)
  if (cur?.draft && messages.value.length === 0 && runs.value.length === 0) return
  const key = newSessionKey()
  // 新建即登记：立即写入索引并持久化，未提交的会话也可见、可恢复（评审 1.2）
  sessions.value.unshift({ key, title: t('agent.session_new'), updatedAt: Date.now(), draft: true })
  persistSessions()
  activeKey.value = key
  entities.value = []
  skillPath.value = null
  instruction.value = ''
  selectedModel.value = null
  messages.value = []
  runs.value = []
  liveMessages.value = []
  historySnapshot.value = []
  submittedRunId.value = null
  cancelling.value = false
  void loadChat()
  nextTick(() => textareaRef.value?.focus())
}

async function handleDeleteSession(key: string) {
  // 检查该会话是否有活跃 run（pending/running）：删除 = 移除会话文件 + 全部 run 记录，
  // 若正在运行，pi 进程会继续烧 token 直到自然结束或超时，产出写入已删除记录后静默丢弃。
  // 用户直觉是「删除=停止」，因此先提示「将同时停止」，后端 delete_agent_session 统一
  // 先取消活跃 run 再删除。
  let activeRunForSession: AgentRunSummary | undefined
  try {
    const sessionRuns = await listAgentRuns(key, 50)
    activeRunForSession = sessionRuns.find((r) => r.status === 'pending' || r.status === 'running')
  } catch {
    // 查询失败不阻塞删除（按无活跃 run 处理）
  }
  const confirmed = await confirm(
    activeRunForSession ? t('agent.delete_session_running_confirm') : t('agent.delete_session_confirm'),
    {
      title: t('agent.delete_session'),
      kind: 'warning',
    },
  )
  if (!confirmed) return
  try {
    // 后端统一处理：先取消活跃 run（若有），再删除会话记录
    await deleteAgentSession(key)
    const idx = sessions.value.findIndex((s) => s.key === key)
    if (idx >= 0) sessions.value.splice(idx, 1)
    persistSessions()
    if (key === activeKey.value) {
      // 全部会话删除后：立即登记一个新草稿会话（activeKey 恒对应索引中的一项）
      if (sessions.value.length === 0) {
        sessions.value = [{ key: newSessionKey(), title: t('agent.session_new'), updatedAt: Date.now(), draft: true }]
        persistSessions()
      }
      activeKey.value = sessions.value[0].key
      entities.value = []
      skillPath.value = null
      instruction.value = ''
      selectedModel.value = sessions.value.find((s) => s.key === activeKey.value)?.model ?? null
      await loadChat()
    }
    showToast(t('agent.session_deleted'))
  } catch (e) {
    showToast(String(e))
  }
}

// 一键清理：删除除当前会话外的全部历史会话（文件 + DB 记录），带确认
async function handleClearSessions() {
  const targets = sessions.value.filter((s) => s.key !== activeKey.value)
  if (targets.length === 0) return
  // 检查目标会话中是否有活跃 run：清理同样会删除运行记录，正在跑的 run 会继续烧 token
  // 直到自然结束或超时，产出写入已删除记录后静默丢弃——先提示并同时停止。
  let runningCount = 0
  try {
    for (const s of targets) {
      const sessionRuns = await listAgentRuns(s.key, 50)
      const active = sessionRuns.find((r) => r.status === 'pending' || r.status === 'running')
      if (active) {
        runningCount++
      }
    }
  } catch {
    // 查询失败不阻塞清理（按无活跃 run 处理）
  }
  const confirmed = await confirm(
    runningCount > 0 ? t('agent.clear_sessions_running_confirm', String(runningCount)) : t('agent.clear_sessions_confirm'),
    {
      title: t('agent.session_clear'),
      kind: 'warning',
    },
  )
  if (!confirmed) return
  let failed = 0
  // 后端 delete_agent_session 统一处理：先取消活跃 run（若有），再删除会话记录
  for (const s of targets) {
    try {
      await deleteAgentSession(s.key)
    } catch {
      failed++
    }
  }
  sessions.value = sessions.value.filter((s) => s.key === activeKey.value)
  persistSessions()
  if (failed > 0) showToast(t('agent.clear_sessions_partial', String(failed)))
  else showToast(t('agent.sessions_cleared', String(targets.length - failed)))
}

// ── 数据源：全局 skill 列表 + 实体目录（[[]] 菜单/名称映射）────
const skills = ref<string[]>([])
const sources = ref<Source[]>([])
const releases = ref<ReleaseInfo[]>([])

// ── 聊天状态 ──
const messages = ref<AgentChatMessage[]>([])
const messagesLoading = ref(false)
const entities = ref<AgentEntityRefSeed[]>([])
const skillPath = ref<string | null>(null)
const instruction = ref('')
const submitting = ref(false)

// ── 会话上下文水位（评审 P1：上下文水位可见性）──
const usage = ref<AgentSessionUsage | null>(null)
// 警告阈值（字符）：约 10 万 tokens 的中高水位（中文 token ≈ 字符数/2）。
// 模型上下文大小不一（128k~200k tokens），取保守中位，接近即提示开新会话。
const USAGE_WARN_CHARS = 200_000

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
  const tokens = Math.max(1, Math.round(u.total_chars / 2))
  return t('agent.context_usage', String(u.message_count), String(tokens))
})
const usageWarn = computed<boolean>(() => (usage.value?.total_chars ?? 0) > USAGE_WARN_CHARS)

// ── 运行历史面板（评审 P1：耗时 / 模型 / 状态 / 引用实体）──
const historyOpen = ref(false)

function runDurationText(run: AgentRunSummary): string {
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

function runModelLabel(run: AgentRunSummary): string {
  const m = runModel(run)
  return m ? m.model_id : t('agent.run_model_default')
}

function runEntityCount(run: AgentRunSummary): number {
  return runEntities(run).length
}

// ── 超时引导（评审 P1：行动建议 + 就地调时长）──
const timeoutSecs = ref(300)
const adjustingTimeout = ref(false)
const timeoutInput = ref('')

function isTimeoutRun(run: AgentRunSummary | undefined): boolean {
  return run?.status === 'timeout'
}

/** 终态判定（success / failed / timeout / cancelled）：历史面板仅对终态 run 展示「重试」。 */
function isTerminalRun(run: AgentRunSummary): boolean {
  return run.status !== 'pending' && run.status !== 'running'
}

async function saveTimeout() {
  const v = Number(timeoutInput.value)
  if (!Number.isInteger(v) || v < 10 || v > 3600) {
    showToast(t('agent.timeout_range'))
    return
  }
  try {
    const cfg = await getAgentConfig()
    cfg.timeout_seconds = v
    await saveAgentConfig(cfg)
    timeoutSecs.value = v
    adjustingTimeout.value = false
    showToast(t('agent.timeout_saved', String(v)))
  } catch (e) {
    showToast(String(e))
  }
}

// ── 模型选择：scope model（pi 已配置鉴权可用）+ 当前激活模型（「默认」落点）──
// selectedModel 按会话记住（存 SessionMeta.model）；null =「默认 - 跟随 pi 当前」。
const availableModels = ref<RpcAvailableModel[]>([])
const currentModel = ref<RpcAvailableModel | null>(null)
const selectedModel = ref<AgentModelRef | null>(null)

// ── 引用 chip 全文悬浮提示（仅文本被截断时显示，跟随鼠标）──
const chipTooltip = ref<{ x: number; y: number; text: string } | null>(null)

function chipTextTruncated(el: HTMLElement): boolean {
  return el.scrollWidth > el.clientWidth + 1
}

function placeChipTooltip(x: number, y: number, text: string) {
  const maxWidth = 480
  const margin = 16
  const left = Math.max(margin, Math.min(x + 12, window.innerWidth - maxWidth - margin))
  chipTooltip.value = { x: left, y: y + 12, text }
}

function handleChipEnter(e: MouseEvent, text: string) {
  const el = e.currentTarget as HTMLElement
  if (!chipTextTruncated(el)) return
  placeChipTooltip(e.clientX, e.clientY, text)
}

function handleChipMove(e: MouseEvent) {
  if (!chipTooltip.value) return
  placeChipTooltip(e.clientX, e.clientY, chipTooltip.value.text)
}

function hideChipTooltip() {
  chipTooltip.value = null
}
const runs = ref<AgentRunSummary[]>([])
const textareaRef = ref<HTMLTextAreaElement | null>(null)
const scrollRef = ref<HTMLElement | null>(null)
// 当前会话活跃 run：优先取 running（真正占用进程执行的那条），无 running 才取
// pending（最新排队的那条）。runs 是倒序（ORDER BY id DESC），直接 find pending/running
// 会取到「最新排队」而非「正在执行」——若同会话异常出现多活跃 run，点「停止」会
// 停掉排队的、执行中的继续跑（评审 3.2）。从 runs 推导而非独立字段：切换/新建会话
// 后 loadChat 即恢复停止能力，不依赖流式事件回填（无输出的 run 也能停），
// 后端排队中的 run 同样可停。
const activeRun = computed<AgentRunSummary | undefined>(() =>
  runs.value.find((r) => r.status === 'running') ?? runs.value.find((r) => r.status === 'pending'),
)
// 提交回执兜底：run 刚创建、runs 尚未刷新时保持可停止（等价旧逻辑「run 未在列表视为运行中」）
const submittedRunId = ref<number | null>(null)
const activeRunId = computed<number | null>(() => activeRun.value?.id ?? submittedRunId.value)
// 是否处于可停止状态：会话内有活跃 run（运行中或排队中）
const canStop = computed(() => activeRunId.value !== null)
const cancelling = ref(false)
// 横幅快捷操作（在 Agent 中打开 / 复制命令）折叠状态：默认折叠节省空间，点击 << 展开、>> 收起
const actionsExpanded = ref(false)

// ── 流式渲染：RPC 事件实时追加的消息（终态后 loadChat 全量校准）──
const liveMessages = ref<AgentChatMessage[]>([])
// 提交时刻的历史快照：流式期间显示「快照历史 + 流式内容」，
// 保证 AI 生成中前文始终可见（与 pi GUI 一致）；终态后清空回落全量校准。
const historySnapshot = ref<AgentChatMessage[]>([])
// 流式进行中 = 历史快照 + 实时流式（不再二选一丢弃历史）；
// 终态（agent_settled）清空流式后回落全量校准结果。
const displayedMessages = computed(() =>
  liveMessages.value.length > 0
    ? [...historySnapshot.value, ...liveMessages.value]
    : messages.value,
)

/** 当前流式 assistant 消息（没有则创建一条）。 */
function liveAssistant(): AgentChatMessage {
  let cur = liveMessages.value[liveMessages.value.length - 1]
  if (!cur || cur.role !== 'assistant') {
    cur = { role: 'assistant', blocks: [], timestamp: new Date().toISOString(), model: null }
    liveMessages.value.push(cur)
  }
  return cur
}

/** 处理 pi RPC 事件流（打字机文本 / 工具状态 / 流式 bash 输出）。 */
function handleRpcStream(payload: { session_key: string; run_id: number; event: string }) {
  if (payload.session_key !== activeKey.value) return
  let ev: Record<string, unknown>
  try {
    ev = JSON.parse(payload.event) as Record<string, unknown>
  } catch {
    return
  }
  if (!ev || typeof ev.type !== 'string') return
  const type = ev.type

  if (type === 'message_update') {
    const ae = ev.assistantMessageEvent as { type?: string; delta?: string } | undefined
    if (!ae?.delta) return
    const cur = liveAssistant()
    if (ae.type === 'text_delta') {
      const last = cur.blocks[cur.blocks.length - 1]
      if (last && last.kind === 'text') {
        ;(last as { kind: 'text'; text: string }).text += ae.delta
      } else {
        cur.blocks.push({ kind: 'text', text: ae.delta })
      }
    } else if (ae.type === 'thinking_delta') {
      const last = cur.blocks[cur.blocks.length - 1]
      if (last && last.kind === 'thinking') {
        ;(last as { kind: 'thinking'; text: string }).text += ae.delta
      } else {
        cur.blocks.push({ kind: 'thinking', text: ae.delta })
      }
    }
  } else if (type === 'tool_execution_start') {
    const cur = liveAssistant()
    cur.blocks.push({
      kind: 'toolCall',
      id: String(ev.toolCallId ?? ''),
      name: String(ev.toolName ?? ''),
      args: JSON.stringify(ev.args ?? {}),
    })
  } else if (type === 'bash_execution_update') {
    const delta = typeof ev.delta === 'string' ? ev.delta : ''
    if (!delta) return
    const cur = liveAssistant()
    const last = cur.blocks[cur.blocks.length - 1]
    if (last && last.kind === 'bash') {
      ;(last as { kind: 'bash'; output: string }).output += delta
    } else {
      cur.blocks.push({ kind: 'bash', command: '', output: delta, exit_code: null, truncated: false })
    }
  } else if (type === 'agent_settled') {
    // 整轮完成：停轮询 + 清流式/快照 + 全量校准（与 JSONL 一致）
    stopPolling()
    liveMessages.value = []
    historySnapshot.value = []
    void loadChat()
  }
}

// 流式期间跟随滚动（仅当用户停留在底部附近时）
watch(liveMessages, () => scrollToBottomIfNear(), { deep: true })

// ── 引用菜单状态 ──
const showSkillMenu = ref(false)
const skillQuery = ref('')
const showEntityMenu = ref(false)
const entityQuery = ref('')
const skillMenuIndex = ref(0)
const entityMenuIndex = ref(0)
const showModelMenu = ref(false)
const modelMenuIndex = ref(0)

let unlistenRunFinished: UnlistenFn | undefined
let unlistenRpcStream: UnlistenFn | undefined
let pollTimer: ReturnType<typeof setInterval> | undefined
let pollDeadline: number | undefined

// ── 事件桥：主窗口「发送到 Agent」/ 拖拽热区 → 加入引用 ──
function addEntity(e: AgentEntityRefSeed) {
  if (!entities.value.some((x) => x.kind === e.kind && x.id === e.id)) {
    entities.value.push(e)
  }
}

function removeEntity(index: number) {
  entities.value.splice(index, 1)
}

function entityLabel(e: AgentEntityRefSeed): string {
  if (e.kind === 'source') {
    const s = sources.value.find((x) => x.id === e.id)
    return s ? `${s.source_type} | ${sourceDisplayName(s)}` : `source #${e.id}`
  }
  const r = releases.value.find((x) => x.id === e.id)
  return r ? releaseDisplayName(r) : `release #${e.id}`
}

function entityKindLabel(kind: string): string {
  return kind === 'source' ? t('agent.entity_source') : t('agent.entity_release')
}

// ── 加载：会话记录 + 聊天消息 ──
async function loadRuns() {
  try {
    runs.value = await listAgentRuns(activeKey.value, 50)
  } catch {
    runs.value = []
  }
}

async function loadMessages() {
  try {
    messages.value = await listAgentMessages(activeKey.value)
  } catch {
    // 会话文件读取失败不阻塞（新会话为空）
  }
}

// ── 全局队列状态（「排队中」提示：其他会话占用 + 队列位置）──
const queueInfo = ref<AgentQueueStatus | null>(null)

async function loadQueueInfo() {
  try {
    queueInfo.value = await getAgentQueueStatus(activeKey.value)
  } catch {
    queueInfo.value = null
  }
}

// ── 全局队列（侧栏运行状态点 / 横幅「被谁占用」）：全部活跃 run 按执行顺序 ──
const queueActive = ref<AgentQueueItem[]>([])

async function loadQueue() {
  try {
    queueActive.value = await getAgentQueue()
  } catch {
    queueActive.value = []
  }
}

/** 某会话的运行状态点：running（执行中）优先，否则取队列最前的 pending。 */
function sessionRunState(key: string): { status: string; position: number } | null {
  const items = queueActive.value.filter((i) => i.session_key === key)
  if (items.length === 0) return null
  const running = items.find((i) => i.status === 'running')
  if (running) return { status: 'running', position: running.position }
  return { status: 'pending', position: items[0].position }
}

/** 侧栏渲染源：sessions 预附运行状态（每项只算一次，避免模板内重复调用 sessionRunState）。 */
const sessionsWithState = computed(() =>
  sessions.value.map((s) => ({ ...s, state: sessionRunState(s.key) })),
)

function sessionTitleOf(key: string): string {
  return sessions.value.find((s) => s.key === key)?.title || t('agent.session_untitled')
}

/** 占用执行位的其他会话 key（本会话 pending 且其他会话 running）：横幅可点击跳转。 */
const queueOccupiedBy = computed<string | null>(() => {
  const q = queueInfo.value
  if (!q?.other_running || latestRun.value?.status !== 'pending') return null
  const key = q.running_sessions[0]
  return key && key !== activeKey.value ? key : null
})

/** 横幅「排队中」补充提示：其他会话执行中 / 队列位置。 */
const queueHint = computed<string | null>(() => {
  const q = queueInfo.value
  if (!q || latestRun.value?.status !== 'pending') return null
  if (q.other_running) {
    return q.position && q.position > 1
      ? t('agent.queue_other_running_pos', String(q.position))
      : t('agent.queue_other_running')
  }
  return q.position && q.position > 1 ? t('agent.queue_position', String(q.position)) : null
})

async function loadChat() {
  messagesLoading.value = true
  // 切会话时先清旧水位：loadUsage 异步返回前，避免闪现上一会话的水位条 / 橙色告警
  usage.value = null
  try {
    await Promise.all([loadRuns(), loadMessages(), loadQueueInfo(), loadQueue(), loadUsage()])
    // runs 已刷新：提交兜底使命结束，活跃 run 由 activeRun 推导接管
    submittedRunId.value = null
    // 切回正在运行的会话：把已加载的历史冻结进快照，
    // 新 delta 到达时 displayedMessages = [历史快照, ...流式]，不吞历史。
    // 提交路径（liveMessages 已有回显）与终态路径（activeRun 已消失）不受影响。
    if (activeRun.value && liveMessages.value.length === 0) {
      historySnapshot.value = [...messages.value]
    }
  } finally {
    messagesLoading.value = false
    scrollToBottom()
  }
}

async function loadCatalog() {
  try {
    const [cfg, srcs, rels] = await Promise.all([getAgentConfig(), listSources(), getReleases()])
    skills.value = cfg.skills
    timeoutSecs.value = cfg.timeout_seconds
    sources.value = srcs
    releases.value = rels
  } catch {
    // 目录加载失败不阻塞工作区使用（名称映射降级为 #id）
  }
  // 模型列表独立拉取：失败仅影响模型下拉（只剩「默认」），不影响技能/实体目录
  await loadModels()
}

async function loadModels() {
  try {
    const info = await getAgentAvailableModels()
    availableModels.value = info.models
    currentModel.value = info.current
  } catch {
    availableModels.value = []
    currentModel.value = null
  }
}

// ── 轮询：提交后增量拉取消息，直到运行结束事件 ──
function startPolling() {
  stopPolling()
  pollDeadline = Date.now() + 120_000 // 兜底：120s 后强制停止
  pollTimer = setInterval(async () => {
    if (pollDeadline && Date.now() > pollDeadline) {
      stopPolling()
      return
    }
    // 轮询期顺带刷新全局队列：排队 run 开始/结束时侧栏状态点及时更新（轻量 SQL）
    await Promise.all([loadMessages(), loadQueue()])
    scrollToBottomIfNear()
  }, 1500)
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = undefined
  }
  pollDeadline = undefined
}

// ── 滚动 ──
function scrollToBottom() {
  nextTick(() => {
    const el = scrollRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

function scrollToBottomIfNear() {
  const el = scrollRef.value
  if (!el) return
  const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 160
  if (nearBottom) scrollToBottom()
}

// ── 提交 ──
function parseInlineRefs(text: string): { entities: AgentEntityRefSeed[]; cleaned: string } {
  const found: AgentEntityRefSeed[] = []
  let cleaned = text.replace(/\[\[(source|release):(\d+)\]\]/g, (_m, kind: string, id: string) => {
    found.push({ kind: kind as 'source' | 'release', id: Number(id) })
    return ''
  })
  // 已选 Skill：移除输入框里的 @短名 标记（skillPath 独立提交，消息区仅用徽章展示，避免重复）
  if (skillPath.value) {
    const name = skillShortName(skillPath.value)
    cleaned = cleaned.replace(new RegExp(`@${escapeRegExp(name)}\\s*`), '')
  }
  return { entities: found, cleaned }
}

async function handleSubmit() {
  // 菜单打开时 Enter 用于选择菜单项，不触发提交
  if (showSkillMenu.value || showEntityMenu.value) return
  const { entities: inline, cleaned } = parseInlineRefs(instruction.value)
  const all = [...entities.value, ...inline]
  const merged: AgentEntityRefSeed[] = []
  for (const e of all) {
    if (!merged.some((x) => x.kind === e.kind && x.id === e.id)) merged.push(e)
  }
  if (cleaned.trim() === '' && merged.length === 0) {
    showToast(t('agent.empty_job'))
    return
  }
  submitting.value = true
  try {
    // 新 run 开始：冻结历史快照 + 本地回显用户消息（pi 落盘有延迟），
    // 流式期间界面显示「历史 + 用户消息 + AI 实时输出」；
    // 终态由 agent_settled 统一清流式并全量校准（与 JSONL 一致）。
    historySnapshot.value = [...messages.value]
    liveMessages.value = [
      {
        role: 'user',
        blocks: [{ kind: 'text', text: cleaned.trim() }],
        timestamp: new Date().toISOString(),
        model: null,
      },
    ]
    const runId = await runAgentJob({
      sessionKey: activeKey.value,
      entities: merged,
      skillPath: skillPath.value,
      instruction: cleaned.trim(),
      model: selectedModel.value,
    })
    submittedRunId.value = runId
    track('agent.submit')
    instruction.value = ''
    // 会话登记（标题取首次指令前 40 字）+ 固化本次模型选择 + 清除草稿标记
    // （新建即登记后 key 恒在索引中；draft 清除 = 已提交，不再是「新会话」）
    const now = Date.now()
    const idx = sessions.value.findIndex((s) => s.key === activeKey.value)
    const title = cleaned.trim() ? [...cleaned.trim()].slice(0, 40).join('') : sessionTitle.value
    if (idx >= 0) {
      sessions.value[idx] = { ...sessions.value[idx], title, updatedAt: now, model: selectedModel.value, draft: false }
    } else {
      sessions.value.unshift({ key: activeKey.value, title, model: selectedModel.value, updatedAt: now, draft: false })
    }
    persistSessions()
    await loadChat()
    startPolling()
  } catch (e) {
    showToast(String(e))
    // 提交被拒（未创建 run，不会有 AgentRunFinished 事件兜底）：
    // 清掉本地回显与历史快照，避免失败消息永久滞留成「幽灵消息」
    liveMessages.value = []
    historySnapshot.value = []
  } finally {
    submitting.value = false
  }
}

// ── 停止：中断当前 run（kill 子进程树，终态 cancelled）──
async function handleCancel() {
  if (activeRunId.value === null || cancelling.value) return
  cancelling.value = true
  try {
    await cancelAgentRun(activeRunId.value)
    showToast(t('agent.cancelling'))
    startPolling() // 等终态事件刷新；事件丢失时轮询兜底
  } catch (e) {
    showToast(String(e))
    cancelling.value = false
  }
}

// ── 消息渲染辅助 ──
// 时间窗对位（fallback）：runs（倒序）按顺序对位 user 消息。runForMessage 优先用
// 后端直连的 msg.run_id（list_agent_messages 按创建顺序填好，见其命令注释），
// 此处仅兜底旧数据 / 后端异常未填充的场景。
const userRunMap = computed<Map<number, AgentRunSummary>>(() => {
  const map = new Map<number, AgentRunSummary>()
  const runsAsc = [...runs.value].reverse()
  let userIdx = 0
  for (let i = 0; i < messages.value.length; i++) {
    const m = messages.value[i]
    if (m.role !== 'user') continue
    const run = runsAsc[userIdx]
    if (run) {
      const runMs = new Date(run.created_at).getTime()
      const msgMs = new Date(m.timestamp).getTime()
      if (Number.isFinite(runMs) && Number.isFinite(msgMs) && Math.abs(runMs - msgMs) < 60_000) {
        map.set(i, run)
      }
    }
    userIdx++
  }
  return map
})

/** 某条 user 消息对应的 run：优先 run_id 直连（后端 list_agent_messages 已按创建
 * 顺序对位并带 started_at 邻近校验——run 未产生消息的路径如排队中取消/派发前失败
 * 已被跳过，评审 1.5 + 复核）。直连命中后仍校验 started_at 与消息时间邻近
 * （防御旧版本绑定/后端异常），不通过落回 60 秒时间窗兜底——双层拒绝错挂。 */
function runForMessage(idx: number): AgentRunSummary | undefined {
  const msg = messages.value[idx]
  if (msg?.run_id) {
    const byId = runs.value.find((r) => r.id === msg.run_id)
    if (byId && byId.started_at && timeAdjacent(byId.started_at, msg.timestamp)) return byId
  }
  return userRunMap.value.get(idx)
}

/** 两个 RFC3339 时间是否在 60 秒窗内（run_id 直连与时间窗兜底的共同校验）。 */
function timeAdjacent(a: string, b: string): boolean {
  const ta = new Date(a).getTime()
  const tb = new Date(b).getTime()
  return Number.isFinite(ta) && Number.isFinite(tb) && Math.abs(ta - tb) < 60_000
}

/** 最近一次 run（状态横幅用）。 */
const latestRun = computed<AgentRunSummary | undefined>(() => runs.value[0])

function runStatusLabel(status: string): string {
  return t(`agent.status_${status}`)
}

function blockText(blocks: { kind: string; text?: string }[]): string {
  return blocks
    .filter((b) => b.kind === 'text')
    .map((b) => b.text ?? '')
    .join('\n')
}

function toolArgsSummary(args: string): string {
  const t0 = args.trim()
  if (!t0) return t0
  return t0.length > 120 ? t0.slice(0, 120) + '…' : t0
}

// ── 运行操作（终端恢复，高级功能保留）──
async function handleOpenSession(run: AgentRunSummary) {
  if (!run.session_path) return
  try {
    await openAgentSession(run.id)
  } catch (e) {
    showToast(String(e))
  }
}

async function handleCopySessionCommand(run: AgentRunSummary) {
  if (!run.session_path) return
  try {
    const cmd = await getAgentSessionCommand(run.id)
    await navigator.clipboard.writeText(cmd)
    showToast(t('agent.command_copied'))
  } catch (e) {
    showToast(String(e))
  }
}

// ── @ 选择 Skill / [[ 引用实体 ──
const SKILL_TRIGGER = /@([\w\-.\\/]*)$/
const ENTITY_TRIGGER = /\[\[([^\]]*)$/

function handleInput() {
  const el = textareaRef.value
  if (!el) return
  const before = el.value.slice(0, el.selectionStart)
  const skillMatch = before.match(SKILL_TRIGGER)
  const entityMatch = before.match(ENTITY_TRIGGER)
  showModelMenu.value = false
  if (skillMatch && !entityMatch) {
    skillQuery.value = skillMatch[1]
    skillMenuIndex.value = 0
    showSkillMenu.value = true
    showEntityMenu.value = false
  } else if (entityMatch) {
    entityQuery.value = entityMatch[1]
    entityMenuIndex.value = 0
    showEntityMenu.value = true
    showSkillMenu.value = false
  } else {
    showSkillMenu.value = false
    showEntityMenu.value = false
  }
}

const filteredSkills = computed(() => {
  const q = skillQuery.value.toLowerCase()
  return skills.value.filter((s) => skillShortName(s).toLowerCase().includes(q) || s.toLowerCase().includes(q))
})

// [[ 实体：无前缀时两类都模糊搜；s: / r: 前缀限定类型
const filteredSources = computed(() => {
  const q = entityQuery.value.toLowerCase()
  if (q.startsWith('r:')) return []
  const name = q.startsWith('s:') ? q.slice(2) : q
  return sources.value.filter((s) =>
    `${s.owner}/${s.repo} ${s.source_type} ${s.description ?? ''}`.toLowerCase().includes(name),
  )
})

const filteredReleases = computed(() => {
  const q = entityQuery.value.toLowerCase()
  if (q.startsWith('s:')) return []
  const name = q.startsWith('r:') ? q.slice(2) : q
  return releases.value
    .filter((r) =>
      `${r.owner}/${r.repo} ${r.tag_name} ${r.release_name} ${r.source_description ?? ''}`
        .toLowerCase()
        .includes(name),
    )
    .slice(0, 30)
})

const filteredSourcesCount = computed(() => filteredSources.value.length)
const filteredReleasesCount = computed(() => filteredReleases.value.length)
const entityMenuHasMatch = computed(() => filteredSourcesCount.value > 0 || filteredReleasesCount.value > 0)

/** 菜单项可读名（可读名优先，回退 ID）：
 * - source：YouTube 等源的 description 存真实频道名；
 * - release：频道/仓库名 + 版本/视频标题。 */
function sourceDisplayName(s: Source): string {
  const name = s.description?.trim()
  return name && name.length > 0 ? name : s.owner
}
function releaseDisplayName(r: ReleaseInfo): string {
  const title = r.release_name && r.release_name !== r.tag_name ? r.release_name : r.tag_name
  const channel = r.source_description?.trim()
  return `${channel && channel.length > 0 ? channel : `${r.owner}/${r.repo}`} · ${title}`
}

function replaceTrigger(replacement: string) {
  const el = textareaRef.value
  if (!el) return
  const before = el.value.slice(0, el.selectionStart)
  const after = el.value.slice(el.selectionStart)
  const skillMatch = before.match(SKILL_TRIGGER)
  const entityMatch = before.match(ENTITY_TRIGGER)
  const start =
    skillMatch && !entityMatch
      ? before.length - skillMatch[1].length - 1
      : entityMatch
        ? before.length - entityMatch[1].length - 2
        : before.length
  el.value = el.value.slice(0, start) + replacement + after
  instruction.value = el.value
  const pos = start + replacement.length
  el.setSelectionRange(pos, pos)
  el.focus()
}

function pickSkill(path: string) {
  skillPath.value = path
  // 输入框只插入短名（带 @ 前缀所见即所得；skillPath 独立字段携带完整路径提交）
  replaceTrigger(`@${skillShortName(path)} `)
  showSkillMenu.value = false
}

function clearSkill() {
  skillPath.value = null
}

// ── 模型下拉 ──
/** 模型可读名（name 优先，回退 id）。 */
function modelLabel(m: RpcAvailableModel | AgentModelRef): string {
  const name = 'name' in m ? (m as RpcAvailableModel).name : undefined
  const id = 'model_id' in m ? m.model_id : (m as RpcAvailableModel).id
  return name && name.length > 0 ? name : id
}
/** 唯一键（provider + modelId，modelId 可能自带 provider 前缀，故不拼接 id）。 */
function modelKey(m: RpcAvailableModel | AgentModelRef): string {
  const id = 'model_id' in m ? m.model_id : (m as RpcAvailableModel).id
  return `${m.provider}\u0000${id}`
}
function isModelSelected(m: RpcAvailableModel): boolean {
  return selectedModel.value?.provider === m.provider && selectedModel.value.model_id === m.id
}
/** 下拉按钮展示：显式选择 → 选中模型名；否则「默认」+ pi 当前模型名。 */
const activeModelLabel = computed<string>(() => {
  if (selectedModel.value) return modelLabel(selectedModel.value)
  return currentModel.value ? modelLabel(currentModel.value) : t('agent.model_default')
})
/** 「默认」副标题：当前 pi 实际将用的模型（provider · id）。 */
const modelDefaultSub = computed<string>(() =>
  currentModel.value ? `${currentModel.value.provider} · ${currentModel.value.id}` : '',
)

function toggleModelMenu() {
  if (showModelMenu.value) {
    showModelMenu.value = false
    return
  }
  showSkillMenu.value = false
  showEntityMenu.value = false
  showModelMenu.value = true
  modelMenuIndex.value = selectedModel.value ? availableModels.value.findIndex(isModelSelected) + 1 : 0
}

/** 点菜单及其触发控件之外任意区域 → 收起当前打开的菜单（下拉菜单通用行为）。 */
function onDocumentPointerDown(e: MouseEvent | PointerEvent) {
  // 通过捕获期触发，确保在菜单项 @click 之前执行；只判断是否点到了「菜单或触发控件」内部，
  // 是则交由原逻辑（切换/选择）处理，否则一律收起，实现点击空白区域收起。
  const t = e.target as EventTarget | null
  if (!(t instanceof Element)) return
  if (t.closest('.agent-ws-menu')) return
  if (t.closest('.agent-ws-model-btn')) return // 模型菜单的触发按钮，交给 toggleModelMenu
  if (t.closest('.agent-ws-textarea')) return // 技能/实体菜单跟随输入，点击输入框不干扰
  if (showModelMenu.value) showModelMenu.value = false
  if (showSkillMenu.value) showSkillMenu.value = false
  if (showEntityMenu.value) showEntityMenu.value = false
}

/** 选模型：写入当前会话 meta（按会话记住）。null = 默认（跟随 pi 当前）。 */
function pickModel(m: RpcAvailableModel | null) {
  selectedModel.value = m ? { provider: m.provider, model_id: m.id } : null
  showModelMenu.value = false
  const now = Date.now()
  const idx = sessions.value.findIndex((s) => s.key === activeKey.value)
  if (idx >= 0) {
    sessions.value[idx] = { ...sessions.value[idx], updatedAt: now, model: selectedModel.value }
  } else {
    sessions.value.unshift({ key: activeKey.value, title: sessionTitle.value, model: selectedModel.value, updatedAt: now })
  }
  persistSessions()
}

function pickEntity(kind: 'source' | 'release', id: number) {
  replaceTrigger(`[[${kind}:${id}]] `)
  showEntityMenu.value = false
}

// ── 键盘统一入口 ──
// 菜单打开时：按键先交给菜单（Enter/Tab 选择项、Escape 关闭），一律不触发提交；
// 无菜单时：无修饰键 Enter 提交。避免「选菜单项的同时消息被自动发出」。
function handleKeydown(e: KeyboardEvent) {
  // 输入法组合期（中文候选词确认回车）不触发提交/菜单导航
  if (e.isComposing) return
  if (showSkillMenu.value || showEntityMenu.value || showModelMenu.value) {
    handleMenuKeydown(e)
    return
  }
  if (e.key === 'Enter' && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
    e.preventDefault()
    // 运行中/排队中/提交中：按钮已变为「停止」或禁用，Enter 若继续提交会与按钮
    // 语义矛盾（显式「停止」与隐式「排队新 run」并存）。与按钮一致：禁止提交并提示。
    if (canStop.value || submitting.value) {
      showToast(t('agent.enter_while_running'))
      return
    }
    void handleSubmit()
  }
}

// ── 菜单键盘导航 ──
function handleMenuKeydown(e: KeyboardEvent) {
  if (showModelMenu.value) {
    const total = 1 + availableModels.value.length
    if (total > 1) {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        modelMenuIndex.value = (modelMenuIndex.value + 1) % total
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        modelMenuIndex.value = (modelMenuIndex.value - 1 + total) % total
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        if (modelMenuIndex.value === 0) {
          pickModel(null)
        } else {
          const m = availableModels.value[modelMenuIndex.value - 1]
          if (m) pickModel(m)
        }
      } else if (e.key === 'Escape') {
        e.preventDefault()
        showModelMenu.value = false
      }
    }
    return
  }
  if (showSkillMenu.value) {
    const total = filteredSkills.value.length
    if (total === 0) return
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      skillMenuIndex.value = (skillMenuIndex.value + 1) % total
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      skillMenuIndex.value = (skillMenuIndex.value - 1 + total) % total
    } else if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault()
      const s = filteredSkills.value[skillMenuIndex.value]
      if (s) pickSkill(s)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      showSkillMenu.value = false
    }
    return
  }
  if (showEntityMenu.value) {
    const total = filteredSourcesCount.value + filteredReleasesCount.value
    if (total === 0) return
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      entityMenuIndex.value = (entityMenuIndex.value + 1) % total
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      entityMenuIndex.value = (entityMenuIndex.value - 1 + total) % total
    } else if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault()
      const i = entityMenuIndex.value
      const s = filteredSources.value.length
      if (i < s) {
        const src = filteredSources.value[i]
        pickEntity('source', src.id)
      } else {
        const rel = filteredReleases.value[i - s]
        if (rel) pickEntity('release', rel.id)
      }
    } else if (e.key === 'Escape') {
      e.preventDefault()
      showEntityMenu.value = false
    }
  }
}

// ── 事件 ──
async function onRunFinished(payload: AgentRunFinished) {
  // 不按 session_key 过滤：当前会话若有活跃 run（pending/running），任意会话的
  // run 结束都可能影响它（其他会话结束 → 本会话排队 run 开始执行；本会话结束 →
  // 横幅/停止按钮收尾）。统一刷新，保证「排队中」横幅在别的会话结束后自动更新。
  if (activeRunId.value === null) return
  stopPolling()
  cancelling.value = false
  // 兜底清理：正常路径 agent_settled 已清；abort / 超时 / 模型错误等
  // 场景下 agent_settled 可能不达，run 终态事件统一收尾
  liveMessages.value = []
  historySnapshot.value = []
  await loadChat()
}

/** 失败原因文案：run.error 形如 `err.agent.timeout|300`（i18n 键|参数）。 */
function runErrorText(run: AgentRunSummary | undefined): string | null {
  if (!run?.error) return null
  const [key, ...args] = run.error.split('|')
  const text = t(key, ...args)
  // i18n 未命中时 t() 原样返回 key：不渲染裸键
  return text === key ? null : text
}

/** 失败 run 的内联备注（failed/timeout 且可解析出文案时返回；否则 null）。
 * 挂在对应 user 消息气泡下，让「哪一轮为什么挂了」在对话流里可追溯——
 * 横幅只展示最近一次 run，历史失败原因不再随新提交成功而消失。 */
function runFailedNote(run: AgentRunSummary | undefined): string | null {
  if (!run) return null
  if (run.status !== 'failed' && run.status !== 'timeout') return null
  return runErrorText(run)
}

/** 该 run 是否值得重试（非成功终态即终点不明：失败 / 超时 / 被取消）。 */
function canRetry(run: AgentRunSummary | undefined): boolean {
  if (!run) return false
  return run.status === 'failed' || run.status === 'timeout' || run.status === 'cancelled'
}

/** run 记录固化的模型选择（JSON 字符串；解析失败按「默认」处理）。 */
function runModel(run: AgentRunSummary): AgentModelRef | null {
  if (!run.model) return null
  try {
    return JSON.parse(run.model) as AgentModelRef
  } catch {
    return null
  }
}

/** 过滤掉已删除的引用实体：重试历史 run 时，被引用的监控源/版本可能已被清理，
 * 而后端对任一实体缺失即整体拒绝（err.agent.entity_missing）——不剔除会让整次
 * 重试直接失败，剔除后指令本身仍然成立。 */
function existingEntities(ents: AgentEntityRefSeed[]): AgentEntityRefSeed[] {
  return ents.filter((e) =>
    e.kind === 'source'
      ? sources.value.some((s) => s.id === e.id)
      : releases.value.some((r) => r.id === e.id),
  )
}

/** 把一次 run 的输入（指令 / 引用实体 / skill / 模型）还原到输入区。
 * run.instruction 已剥离 `[[引用]]`（实体归入 run.entities），直接回填即为等价重发。 */
function applyRunToComposer(run: AgentRunSummary) {
  instruction.value = run.instruction
  const ents = runEntities(run)
  const alive = existingEntities(ents)
  if (alive.length !== ents.length) {
    showToast(t('agent.retry_entities_dropped', String(ents.length - alive.length)))
  }
  entities.value = alive
  skillPath.value = run.skill_path || null
  selectedModel.value = runModel(run)
}

/** 重试：用同一条 run 的输入原样重新提交。 */
async function handleRetry(run: AgentRunSummary) {
  if (canStop.value || submitting.value) {
    showToast(t('agent.retry_blocked'))
    return
  }
  applyRunToComposer(run)
  await handleSubmit()
}

/** 编辑后重试：还原到输入区但不提交，用户改完自己发。 */
function handleRetryEdit(run: AgentRunSummary) {
  if (canStop.value || submitting.value) {
    showToast(t('agent.retry_blocked'))
    return
  }
  applyRunToComposer(run)
  nextTick(() => {
    const el = textareaRef.value
    if (!el) return
    el.focus()
    el.setSelectionRange(el.value.length, el.value.length)
  })
}

// ── 预置实体（右键「发送到 Agent」入口携带）：打开时写入 chips ──
function applySeed() {
  if (!props.seed?.entities?.length) return
  for (const e of props.seed.entities) {
    addEntity(e)
  }
}

// ── 拖拽：实体（监控源/版本）从主界面拖入工作区 → 插入引用 chip ──
const dragOver = ref(false)

function handleDrop(e: DragEvent) {
  dragOver.value = false
  const raw = e.dataTransfer?.getData('application/x-relwatch-entity')
  if (!raw) return
  try {
    const entity = JSON.parse(raw) as AgentEntityRefSeed
    if (entity.kind === 'source' || entity.kind === 'release') {
      addEntity(entity)
      showToast(t('agent.attached'))
    }
  } catch {
    // 非本应用拖入内容，忽略
  }
}

// ── 拖到头部标题栏：切换新会话并把实体引用放进新会话 ──
const headerDropOver = ref(false)

function onHeaderDragLeave(e: DragEvent) {
  // 仅当真正离开标题栏时才取消高亮（子元素间移动不闪烁）
  const el = e.currentTarget as HTMLElement
  if (!el.contains(e.relatedTarget as Node | null)) {
    headerDropOver.value = false
  }
}

function handleDropNewSession(e: DragEvent) {
  dragOver.value = false
  headerDropOver.value = false
  const raw = e.dataTransfer?.getData('application/x-relwatch-entity')
  if (!raw) return
  try {
    const entity = JSON.parse(raw) as AgentEntityRefSeed
    if (entity.kind === 'source' || entity.kind === 'release') {
      startNewSession()
      addEntity(entity)
      showToast(t('agent.attached'))
    }
  } catch {
    // 非本应用拖入内容，忽略
  }
}

onMounted(async () => {
  applySeed()
  await Promise.all([loadCatalog(), loadChat()])
  // 磁盘发现放在首次加载之后：会话文件是索引的兜底来源，索引缺失时补入，
  // 补入的会话不打断当前激活会话（仅侧栏可见）
  const recovered = await discoverSessions()
  if (recovered > 0) showToast(t('agent.sessions_recovered', String(recovered)))
  unlistenRunFinished = await events.agentRunFinished.listen((e) => {
    void onRunFinished(e.payload)
  })
  unlistenRpcStream = await events.agentRpcStream.listen((e) => {
    handleRpcStream(e.payload)
  })
  await nextTick()
  textareaRef.value?.focus()
  // 捕获期监听：点击菜单/触发控件之外的区域即收起打开的下拉菜单
  document.addEventListener('pointerdown', onDocumentPointerDown, true)
})

// 面板打开期间 seed 更新（重复点「发送到 Agent」）：追加新实体
watch(
  () => props.seed,
  () => applySeed(),
)

onUnmounted(() => {
  stopPolling()
  unlistenRunFinished?.()
  unlistenRpcStream?.()
  document.removeEventListener('pointerdown', onDocumentPointerDown, true)
})

// 新消息到达时自动滚动（用户接近底部时）
watch(
  () => messages.value.length,
  () => scrollToBottomIfNear(),
)
</script>

<template>
  <div class="agent-ws" :style="{ width: panelWidth + 'px', flexBasis: panelWidth + 'px' }">
    <!-- 头部标题栏：拖入 = 新建会话并放入引用 -->
    <header
      class="agent-ws-header"
      :class="{ 'drop-over': headerDropOver }"
      @dragenter.prevent="headerDropOver = true"
      @dragover.prevent="headerDropOver = true"
      @dragleave.prevent="onHeaderDragLeave"
      @drop.prevent.stop="handleDropNewSession"
    >
      <div class="agent-ws-title">
        <svg class="agent-ws-title-icon"><use href="/icons.svg#agent-icon" /></svg>
        <span>{{ t('agent.workspace_title') }}</span>
      </div>
      <div class="agent-ws-header-actions">
        <button class="btn-sm" :title="t('agent.session_new')" @click="startNewSession">
          <svg class="agent-ws-btn-icon"><use href="/icons.svg#plus-icon" /></svg>
          <span class="agent-ws-btn-label">{{ t('agent.session_new') }}</span>
        </button>
        <button class="btn-sm agent-ws-sessions-btn" :class="{ active: sidebarOpen }" :title="t('agent.session_list')" @click="toggleSidebar">
          <svg class="agent-ws-btn-icon"><use href="/icons.svg#list-icon" /></svg>
          <span class="agent-ws-btn-label">{{ t('agent.session_list') }}</span>
        </button>
        <button class="agent-ws-close" :title="t('release.detail_close')" @click="emit('close')">
          <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
        </button>
      </div>
      <!-- 拖拽悬停提示：虚线框 + 目的说明 -->
      <div v-if="headerDropOver" class="agent-ws-drop-hint agent-ws-drop-hint-header">{{ t('agent.drop_new_session') }}</div>
    </header>

    <!-- 工作区主体：拖入 = 添加到当前会话 -->
    <div
      class="agent-ws-main"
      :class="{ 'drag-over': dragOver }"
      @dragover.prevent="dragOver = true"
      @dragleave="dragOver = false"
      @drop.prevent="handleDrop"
    >
      <!-- 拖拽悬停提示：虚线框 + 目的说明 -->
      <div v-if="dragOver" class="agent-ws-drop-hint agent-ws-drop-hint-main">{{ t('agent.drop_current_session') }}</div>
      <!-- 左侧：聊天区（会话侧栏折叠时占满全宽） -->
      <section class="agent-ws-chat">
        <!-- 最近 run 状态横幅 -->
        <div v-if="latestRun" class="agent-ws-banner" :class="`status-${latestRun.status}`">
          <span class="agent-ws-banner-status">{{ runStatusLabel(latestRun.status) }}</span>
          <!-- 排队提示：被其他会话占用时可点击 → 一键跳到占用会话（在那里点「停止」让路） -->
          <span
            v-if="latestRun.status === 'pending' && queueHint"
            class="agent-ws-banner-queue"
            :class="{ clickable: !!queueOccupiedBy }"
            :title="queueHint"
            @click="queueOccupiedBy && switchSession(queueOccupiedBy)"
          >{{ queueOccupiedBy ? t('agent.queue_occupied_by', sessionTitleOf(queueOccupiedBy)) : queueHint }}</span>
          <span v-if="runErrorText(latestRun)" class="agent-ws-banner-error" :title="runErrorText(latestRun) ?? ''">{{ runErrorText(latestRun) }}</span>
          <span class="agent-ws-banner-text">{{ latestRun.instruction || sessionTitle }}</span>
          <span v-if="latestRun.status === 'running' || latestRun.status === 'pending'" class="agent-ws-banner-spinner" aria-hidden="true"></span>
          <span class="agent-ws-banner-actions">
            <button class="btn-sm" :class="{ active: historyOpen }" :title="t('agent.run_history_title')" @click="historyOpen = !historyOpen">
              {{ t('agent.run_history_title') }}
            </button>
            <template v-if="latestRun.session_path">
              <template v-if="actionsExpanded">
                <button class="btn-sm" :title="t('agent.open_session')" @click="handleOpenSession(latestRun)">{{ t('agent.open_session') }}</button>
                <button class="btn-sm" :title="t('agent.copy_command_hint')" @click="handleCopySessionCommand(latestRun)">{{ t('agent.copy_command') }}</button>
              </template>
              <button
                class="btn-sm agent-ws-banner-toggle"
                :title="actionsExpanded ? t('agent.collapse_actions') : t('agent.expand_actions')"
                @click="actionsExpanded = !actionsExpanded"
              >{{ actionsExpanded ? '>>' : '<<' }}</button>
            </template>
          </span>
        </div>

        <!-- 会话上下文水位（消息数 / 估算 token；接近上限提示开新会话） -->
        <div v-if="usageText" class="agent-ws-usage" :class="{ warn: usageWarn }">
          <span class="agent-ws-usage-text" :title="usageWarn ? usageText : undefined">{{ usageWarn ? t('agent.context_near_limit') : usageText }}</span>
          <button v-if="usageWarn" class="btn-sm agent-ws-usage-new" :title="usageText ?? ''" @click="startNewSession">{{ t('agent.session_new') }}</button>
        </div>

        <!-- 运行历史面板（浮层）：耗时 / 模型 / 状态 / 引用实体 -->
        <div v-if="historyOpen" class="agent-ws-history">
          <div class="agent-ws-history-head">
            <span class="agent-ws-history-title">{{ t('agent.run_history_title') }}</span>
            <button class="agent-ws-history-close" :title="t('release.detail_close')" @click="historyOpen = false">
              <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
            </button>
          </div>
          <ul class="agent-ws-history-list">
            <li v-for="r in runs" :key="r.id" class="agent-ws-history-item">
              <span class="agent-ws-history-status" :class="`st-${r.status}`">{{ runStatusLabel(r.status) }}</span>
              <span class="agent-ws-history-main">
                <span class="agent-ws-history-instr" :title="r.instruction">{{ r.instruction || sessionTitle }}</span>
                <span class="agent-ws-history-meta">
                  {{ runModelLabel(r) }} · {{ runDurationText(r) }}
                  <template v-if="runEntityCount(r) > 0"> · {{ t('agent.run_entities_n', String(runEntityCount(r))) }}</template>
                </span>
              </span>
              <span class="agent-ws-history-actions">
                <button v-if="isTerminalRun(r)" class="btn-sm" :title="t('agent.retry')" @click="handleRetry(r)">{{ t('agent.retry') }}</button>
              </span>
            </li>
            <li v-if="runs.length === 0" class="agent-ws-history-empty">{{ t('agent.run_history_empty') }}</li>
          </ul>
        </div>

        <!-- 消息区 -->
        <div ref="scrollRef" class="agent-ws-messages">
          <div v-if="messagesLoading && messages.length === 0 && liveMessages.length === 0" class="agent-ws-hint">{{ t('agent.loading') }}</div>
          <div v-else-if="displayedMessages.length === 0" class="agent-ws-hint agent-ws-hint-empty">
            {{ t('agent.workspace_empty') }}
          </div>
          <template v-else>
            <div
              v-for="(msg, idx) in displayedMessages"
              :key="`${idx}-${msg.timestamp}`"
              class="agent-ws-msg-row"
              :class="`role-${msg.role}`"
            >
              <!-- user 消息：右对齐气泡 -->
              <div v-if="msg.role === 'user'" class="agent-ws-bubble agent-ws-bubble-user">
                <div v-if="runForMessage(idx)" class="agent-ws-bubble-meta">
                  <span v-for="e in runEntities(runForMessage(idx))" :key="`${e.kind}:${e.id}`" class="agent-ws-chip">
                    <span
                      class="agent-ws-chip-text"
                      @mouseenter="handleChipEnter($event, `${entityKindLabel(e.kind)} · ${entityLabel(e)}`)"
                      @mousemove="handleChipMove"
                      @mouseleave="hideChipTooltip"
                    >{{ entityKindLabel(e.kind) }} · {{ entityLabel(e) }}</span>
                  </span>
                  <span v-if="runForMessage(idx)?.skill_path" class="agent-ws-skill-badge">@{{ skillShortName(runForMessage(idx)!.skill_path ?? '') }}</span>
                </div>
                <p class="agent-ws-msg-text">{{ splitUserBlocks(msg.blocks).main || '…' }}</p>
                <details v-if="splitUserBlocks(msg.blocks).folded" class="agent-ws-fold agent-ws-fold-prompt">
                  <summary>{{ t('agent.prompt_full') }}</summary>
                  <pre class="agent-ws-fold-body">{{ splitUserBlocks(msg.blocks).folded }}</pre>
                </details>
                <!-- 非成功终态内联备注 + 重试入口：这轮为什么挂了、怎么再来一次，
                     都在对话流里可追溯（横幅只显示最近一次 run） -->
                <div
                  v-if="canRetry(runForMessage(idx))"
                  class="agent-ws-run-failed"
                  :class="{ 'run-cancelled': runForMessage(idx)!.status === 'cancelled' }"
                >
                  <span class="agent-ws-run-failed-status">{{ runStatusLabel(runForMessage(idx)!.status) }}</span>
                  <span class="agent-ws-run-failed-text" :title="runFailedNote(runForMessage(idx)) ?? ''">
                    {{ runFailedNote(runForMessage(idx)) || runStatusLabel(runForMessage(idx)!.status) }}
                  </span>
                  <span class="agent-ws-run-failed-actions">
                    <button class="btn-sm" :title="t('agent.retry')" @click="handleRetry(runForMessage(idx)!)">
                      {{ t('agent.retry') }}
                    </button>
                    <button class="btn-sm" :title="t('agent.retry_edit')" @click="handleRetryEdit(runForMessage(idx)!)">
                      {{ t('agent.retry_edit') }}
                    </button>
                  </span>
                  <!-- 超时引导（评审 3.6）：行动建议 + 就地调时长（timeout 每次调度重读，无需重启进程） -->
                  <template v-if="isTimeoutRun(runForMessage(idx))">
                    <span class="agent-ws-run-advice">{{ t('agent.timeout_advice') }}</span>
                    <span v-if="!adjustingTimeout" class="agent-ws-run-advice-actions">
                      <button class="btn-sm" :title="t('agent.timeout_adjust')" @click="adjustingTimeout = true; timeoutInput = String(timeoutSecs)">
                        {{ t('agent.timeout_adjust') }}
                      </button>
                    </span>
                    <span v-else class="agent-ws-run-advice-adjust">
                      <input
                        v-model="timeoutInput"
                        type="number"
                        min="10"
                        max="3600"
                        class="agent-ws-timeout-input"
                        :placeholder="t('agent.timeout_placeholder')"
                        @keydown.enter.prevent="saveTimeout"
                      />
                      <button class="btn-sm" @click="saveTimeout">{{ t('agent.timeout_save') }}</button>
                      <button class="btn-sm" @click="adjustingTimeout = false">{{ t('agent.timeout_cancel') }}</button>
                    </span>
                  </template>
                </div>
              </div>

              <!-- assistant 消息：左对齐，Markdown + 思考/工具折叠 -->
              <div v-else-if="msg.role === 'assistant'" class="agent-ws-bubble agent-ws-bubble-assistant">
                <div v-if="msg.model" class="agent-ws-bubble-model">{{ msg.model }}</div>
                <template v-for="(block, bi) in msg.blocks" :key="bi">
                  <MarkdownContent v-if="block.kind === 'text'" :content="block.kind === 'text' ? block.text : ''" />
                  <details v-else-if="block.kind === 'thinking'" class="agent-ws-fold agent-ws-fold-thinking">
                    <summary>{{ t('agent.thinking') }}</summary>
                    <pre class="agent-ws-fold-body">{{ block.kind === 'thinking' ? block.text : '' }}</pre>
                  </details>
                  <div v-else-if="block.kind === 'toolCall'" class="agent-ws-tool-card">
                    <div class="agent-ws-tool-head">
                      <svg class="agent-ws-tool-icon"><use href="/icons.svg#terminal-icon" /></svg>
                      <span class="agent-ws-tool-name">{{ block.kind === 'toolCall' ? block.name : '' }}</span>
                      <span class="agent-ws-tool-tag">{{ t('agent.tool_call') }}</span>
                    </div>
                    <details v-if="block.kind === 'toolCall' && block.args">
                      <summary>{{ t('agent.tool_args') }}</summary>
                      <pre class="agent-ws-fold-body">{{ toolArgsSummary(block.kind === 'toolCall' ? block.args : '') }}</pre>
                    </details>
                  </div>
                </template>
              </div>

              <!-- toolResult：折叠卡片 -->
              <div v-else-if="msg.role === 'toolResult' || msg.role === 'bash'" class="agent-ws-tool-card" :class="{ 'tool-error': isToolError(msg) }">
                <div class="agent-ws-tool-head">
                  <svg class="agent-ws-tool-icon"><use href="/icons.svg#terminal-icon" /></svg>
                  <span class="agent-ws-tool-name">{{ toolCardName(msg) }}</span>
                  <span v-if="msg.role === 'bash'" class="agent-ws-tool-tag">{{ bashExitLabel(msg) }}</span>
                  <span v-else class="agent-ws-tool-tag">{{ isToolError(msg) ? t('agent.tool_error') : t('agent.tool_result') }}</span>
                </div>
                <details>
                  <summary>{{ t('agent.tool_detail') }}</summary>
                  <pre class="agent-ws-fold-body">{{ toolCardBody(msg) }}</pre>
                </details>
              </div>

              <!-- 其他（custom 等）：左对齐文本 -->
              <div v-else class="agent-ws-bubble agent-ws-bubble-assistant">
                <p class="agent-ws-msg-text">{{ blockText(msg.blocks) || '…' }}</p>
              </div>
            </div>
          </template>
        </div>

        <!-- 输入区 -->
        <footer class="agent-ws-input">
          <div class="agent-ws-input-meta">
            <span v-for="(e, i) in entities" :key="`${e.kind}:${e.id}`" class="agent-ws-chip agent-ws-chip-attached">
              <span
                class="agent-ws-chip-text"
                @mouseenter="handleChipEnter($event, `${entityKindLabel(e.kind)} · ${entityLabel(e)}`)"
                @mousemove="handleChipMove"
                @mouseleave="hideChipTooltip"
              >{{ entityKindLabel(e.kind) }} · {{ entityLabel(e) }}</span>
              <button class="agent-ws-chip-remove" :title="t('agent.remove_entity')" @click="removeEntity(i)">
                <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
              </button>
            </span>
            <span v-if="skillPath" class="agent-ws-skill-badge" :title="skillPath">
              @{{ skillShortName(skillPath) }}
              <button class="agent-ws-chip-remove" :title="t('agent.clear_skill')" @click="clearSkill">
                <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
              </button>
            </span>
          </div>
          <div class="agent-ws-input-row">
            <textarea
              ref="textareaRef"
              v-model="instruction"
              class="agent-ws-textarea"
              :placeholder="t('agent.placeholder')"
              rows="3"
              @input="handleInput"
              @keydown="handleKeydown"
            ></textarea>
          </div>
          <!-- 底部操作行：模型选择（左）+ 发送/停止（右），共占一行 -->
          <div class="agent-ws-input-actions">
            <button class="agent-ws-model-btn" :class="{ open: showModelMenu }" :title="t('agent.model_pick')" @click="toggleModelMenu">
              <svg class="agent-ws-model-icon" viewBox="0 0 16 16"><path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/></svg>
              <span class="agent-ws-model-label">{{ activeModelLabel }}</span>
              <svg class="agent-ws-model-caret" viewBox="0 0 16 16"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>
            </button>
            <button
              class="btn-primary agent-ws-submit"
              :class="{ 'agent-ws-stop': canStop }"
              :disabled="submitting && !canStop"
              :title="canStop ? t('agent.stop_hint') : ''"
              @click="canStop ? handleCancel() : handleSubmit()"
            >
              {{ canStop ? (cancelling ? t('agent.stopping') : t('agent.stop')) : submitting ? t('agent.running') : t('agent.submit') }}
            </button>

            <!-- 模型选择菜单：定位在操作行上方（bottom:100%），紧贴按钮弹出 -->
            <div v-if="showModelMenu" class="agent-ws-menu agent-ws-menu-model">
              <div class="agent-ws-menu-title">{{ t('agent.model_pick') }}</div>
              <button
                class="agent-ws-menu-item"
                :class="{ selected: !selectedModel }"
                @mouseenter="modelMenuIndex = 0"
                @click="pickModel(null)"
              >
                <span class="agent-ws-menu-main">{{ t('agent.model_default') }}</span>
                <span v-if="modelDefaultSub" class="agent-ws-menu-sub">{{ modelDefaultSub }}</span>
              </button>
              <button
                v-for="(m, i) in availableModels"
                :key="modelKey(m)"
                class="agent-ws-menu-item"
                :class="{ selected: isModelSelected(m) }"
                @mouseenter="modelMenuIndex = i + 1"
                @click="pickModel(m)"
              >
                <span class="agent-ws-menu-main">{{ modelLabel(m) }}</span>
                <span class="agent-ws-menu-sub">{{ m.provider }} · {{ m.id }}</span>
              </button>
              <div v-if="availableModels.length === 0" class="agent-ws-menu-empty">{{ t('agent.model_none') }}</div>
            </div>
          </div>

          <!-- @ Skill 菜单 -->
          <div v-if="showSkillMenu" class="agent-ws-menu">
            <div class="agent-ws-menu-title">{{ t('agent.skill_pick') }}</div>
            <button
              v-for="(s, i) in filteredSkills"
              :key="s"
              class="agent-ws-menu-item"
              :class="{ selected: i === skillMenuIndex }"
              @mouseenter="skillMenuIndex = i"
              @click="pickSkill(s)"
            >
              <span class="agent-ws-menu-main">@{{ skillShortName(s) }}</span>
              <span class="agent-ws-menu-sub">{{ s }}</span>
            </button>
            <div v-if="filteredSkills.length === 0" class="agent-ws-menu-empty">
              {{ skills.length === 0 ? t('agent.no_skills') : t('agent.no_match') }}
            </div>
          </div>

          <!-- [[ 实体菜单 -->
          <div v-if="showEntityMenu" class="agent-ws-menu agent-ws-menu-entities">
            <div class="agent-ws-menu-title">{{ t('agent.entity_pick') }}</div>
            <template v-if="filteredSources.length">
              <div class="agent-ws-menu-group">
                <svg class="agent-ws-menu-group-icon"><use href="/icons.svg#source-icon" /></svg>
                {{ t('agent.entity_source') }} ({{ filteredSourcesCount }})
              </div>
              <button
                v-for="(s, i) in filteredSources"
                :key="`s${s.id}`"
                class="agent-ws-menu-item"
                :class="{ selected: i === entityMenuIndex }"
                @mouseenter="entityMenuIndex = i"
                @click="pickEntity('source', s.id)"
              >
                <span class="agent-ws-menu-main">{{ sourceDisplayName(s) }}</span>
                <span class="agent-ws-menu-sub">{{ s.source_type }} · 监控源 #{{ s.id }}</span>
              </button>
            </template>
            <template v-if="filteredReleases.length">
              <div class="agent-ws-menu-group">
                <svg class="agent-ws-menu-group-icon"><use href="/icons.svg#release-icon" /></svg>
                {{ t('agent.entity_release') }} ({{ filteredReleasesCount }})
              </div>
              <button
                v-for="(r, i) in filteredReleases"
                :key="`r${r.id}`"
                class="agent-ws-menu-item"
                :class="{ selected: filteredSources.length + i === entityMenuIndex }"
                @mouseenter="entityMenuIndex = filteredSources.length + i"
                @click="pickEntity('release', r.id)"
              >
                <span class="agent-ws-menu-main">{{ releaseDisplayName(r) }}</span>
                <span class="agent-ws-menu-sub">{{ formatDate(r.published_at) }}</span>
              </button>
            </template>
            <div v-if="!entityMenuHasMatch" class="agent-ws-menu-empty">{{ t('agent.no_match') }}</div>
          </div>
        </footer>
      </section>

      <!-- 右侧：会话侧栏（可折叠，折叠时聊天区占满全宽） -->
      <aside class="agent-ws-sidebar" :class="{ collapsed: !sidebarOpen }">
        <div class="agent-ws-sidebar-title">{{ t('agent.session_list') }}</div>
        <ul class="agent-ws-session-list">
          <li
            v-for="s in sessionsWithState"
            :key="s.key"
            class="agent-ws-session-item"
            :class="{ active: s.key === activeKey, draft: s.draft }"
            :title="s.title"
            @click="switchSession(s.key)"
          >
            <span class="agent-ws-session-name">
              {{ s.title }}
              <!-- 运行状态点：执行中（蓝）/ 排队第 N 位（橙）——全局队列驱动（评审 1.3） -->
              <span
                v-if="s.state"
                class="agent-ws-session-dot"
                :class="`st-${s.state.status}`"
                :title="s.state.status === 'running' ? t('agent.session_running_hint') : t('agent.session_queued_hint', String(s.state.position))"
              >{{ s.state.status === 'running' ? t('agent.status_running') : t('agent.queue_position', String(s.state.position)) }}</span>
              <span v-if="s.recovered" class="agent-ws-session-badge" :title="t('agent.session_recovered_hint')">
                {{ t('agent.session_recovered') }}
              </span>
            </span>
            <span class="agent-ws-session-time">{{ formatDate(new Date(s.updatedAt).toISOString()) }}</span>
            <button class="agent-ws-session-del" :title="t('agent.delete_session')" @click.stop="handleDeleteSession(s.key)">
              <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
            </button>
          </li>
          <li v-if="sessions.length === 0" class="agent-ws-session-empty">{{ t('agent.session_empty') }}</li>
        </ul>
        <button v-if="sessions.length > 1" class="agent-ws-session-clear" :title="t('agent.session_clear')" @click="handleClearSessions">
          <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
          {{ t('agent.session_clear') }}
        </button>
      </aside>

      <!-- 引用 chip 全文悬浮提示（跟随鼠标，仅文本截断时显示） -->
      <div v-if="chipTooltip" class="agent-ws-chip-tooltip" :style="{ left: chipTooltip.x + 'px', top: chipTooltip.y + 'px' }">
        {{ chipTooltip.text }}
      </div>
    </div>
  </div>
</template>

<script lang="ts">
// ── 纯函数辅助（模块级，供模板调用，无组件状态）──
import type { AgentChatBlock } from '../bindings'

/** 正则转义：skill 短名可能含 . - 等元字符（如 code-review）。 */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** pi 展开 /skill: 命令时把 skill 全文注入 user 消息（<skill name=…>…</skill>）。
 * 折叠为空白：skill 徽章由 run.skill_path 渲染，避免全文刷屏（对齐 pi TUI 的「加载 Skill」提示）。 */
function stripSkillBlock(text: string): string {
  return text.replace(/<skill name="[^"]*"[^>]*>[\s\S]*?<\/skill>\s*/, '')
}

/** 用户气泡显示文本拆分（模块级纯函数）：
 * - main：<用户指令> 标签内的用户真实指令（skill 块已剥离）
 * - folded：标签外的模板脚手架（订阅说明 / 外部数据区 / 不可信声明等）
 * 首轮完整模板不再整段刷屏，折叠为可展开的详情块，完整上下文仍可见；
 * 无标签（旧格式 / 多轮精简）时整段作为主文本，行为不变。 */
function splitUserBlocks(blocks: AgentChatBlock[]): { main: string; folded: string | null } {
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

function isToolError(msg: AgentChatMessage): boolean {
  return msg.blocks.some((b) => b.kind === 'toolResult' && b.is_error)
}

function toolCardName(msg: AgentChatMessage): string {
  for (const b of msg.blocks) {
    if (b.kind === 'toolResult') return b.tool_name
    if (b.kind === 'bash') return 'bash'
  }
  return msg.role
}

function bashExitLabel(msg: AgentChatMessage): string {
  const b = msg.blocks.find((x) => x.kind === 'bash') as Extract<AgentChatBlock, { kind: 'bash' }> | undefined
  return b ? `exit ${b.exit_code ?? '?'}` : ''
}

function toolCardBody(msg: AgentChatMessage): string {
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

function runEntities(run: AgentRunSummary | undefined): AgentEntityRefSeed[] {
  if (!run) return []
  try {
    return JSON.parse(run.entities) as AgentEntityRefSeed[]
  } catch {
    return []
  }
}
</script>

<style scoped>
.agent-ws {
  width: 440px;
  flex: 0 0 440px;
  /* 与 App.vue AGENT_PANEL_MIN_WIDTH 同步：窗口过窄时主界面先被 min-width 710 保护，
   * 剩余压缩不再由面板承担（flex-shrink 默认 1 会把面板压到很窄，且显示与状态宽度脱节） */
  min-width: 280px;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
  /* 左侧分隔线由 App.vue 的 .agent-divider 承担（可拖拽调节宽度），不再重复画边框 */
  overflow: hidden;
}
.agent-ws-main.drag-over {
  outline: 2px dashed var(--accent, #2e6fd0);
  outline-offset: -4px;
}

/* 拖拽悬停提示层：虚线框内居中说明文字，不拦截事件 */
.agent-ws-drop-hint {
  position: absolute;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.5px;
  color: var(--accent, #2e6fd0);
  background: color-mix(in srgb, var(--accent, #2e6fd0) 10%, var(--bg));
  pointer-events: none;
  z-index: 30;
}
/* 标题栏提示：覆盖整栏 */
.agent-ws-drop-hint-header {
  inset: 0;
}
/* 工作区提示：顶部居中悬浮条 */
.agent-ws-drop-hint-main {
  top: 12px;
  left: 50%;
  transform: translateX(-50%);
  padding: 6px 16px;
  border-radius: 999px;
  white-space: nowrap;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.18);
}

/* 头部 */
.agent-ws-header {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  background: var(--bg);
}
/* 拖到标题栏：独立虚线框 */
.agent-ws-header.drop-over {
  outline: 2px dashed var(--accent, #2e6fd0);
  outline-offset: -4px;
}
.agent-ws-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 15px;
  font-weight: 600;
}
.agent-ws-title-icon {
  width: 16px;
  height: 16px;
  color: var(--accent, #2e6fd0);
}
.agent-ws-header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}
/* 头部操作按钮：图标 + 文字统一 flex 布局，图标垂直居中于文字 */
.agent-ws-header-actions .btn-sm {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.agent-ws-btn-icon {
  width: 12px;
  height: 12px;
  flex-shrink: 0;
}
.agent-ws-btn-label {
  font-size: 12px;
}
/* 会话侧栏开关：激活态与侧栏选中态同源 accent 高亮 */
.agent-ws-sessions-btn.active {
  background: rgba(46, 111, 208, 0.12);
  border-color: rgba(46, 111, 208, 0.35);
  color: #2e6fd0;
}
.agent-ws-close {
  border: none;
  background: none;
  cursor: pointer;
  color: var(--text);
  padding: 4px;
  display: flex;
  align-items: center;
  border-radius: 6px;
}
.agent-ws-close:hover {
  background: var(--bg-hover);
}
.agent-ws-close svg {
  width: 13px;
  height: 13px;
}

/* 主体：边栏 + 聊天 */
.agent-ws-main {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
}

/* 会话侧栏：位于聊天区右侧（次要内容靠边，不挡主界面与对话之间）
 * collapsed 时宽度收缩为 0，聊天区占满整个工作区 */
.agent-ws-sidebar {
  width: 140px;
  flex-shrink: 0;
  border-left: 1px solid var(--border);
  background: var(--bg-subtle);
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  transition: width 0.18s ease, opacity 0.18s ease, border-left-width 0.18s ease;
}
.agent-ws-sidebar.collapsed {
  width: 0;
  opacity: 0;
  border-left-width: 0;
}
.agent-ws-sidebar > * {
  flex-shrink: 0;
}
.agent-ws-sidebar .agent-ws-session-list {
  flex: 1;
  min-height: 0;
}
.agent-ws-sidebar-title {
  padding: 10px 12px 6px;
  font-size: 11px;
  font-weight: 600;
  opacity: 0.55;
  letter-spacing: 0.04em;
}
.agent-ws-session-list {
  list-style: none;
  margin: 0;
  padding: 4px 6px 10px;
  overflow-y: auto;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.agent-ws-session-item {
  position: relative;
  padding: 7px 8px;
  border-radius: 7px;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border: 1px solid transparent;
}
.agent-ws-session-item:hover {
  background: var(--bg-hover);
}
.agent-ws-session-item.active {
  background: rgba(46, 111, 208, 0.12);
  border-color: rgba(46, 111, 208, 0.35);
}
.agent-ws-session-name {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  padding-right: 16px;
}
/* 未提交草稿会话（新建即登记，评审 1.2）：弱化样式以示「还没对话」 */
.agent-ws-session-item.draft .agent-ws-session-name {
  opacity: 0.65;
  font-style: italic;
}
/* 运行状态点：执行中（蓝）/ 排队第 N 位（橙），全局队列驱动（评审 1.3） */
.agent-ws-session-dot {
  display: inline-block;
  margin-left: 4px;
  padding: 0 4px;
  font-size: 9px;
  line-height: 14px;
  border-radius: 3px;
  vertical-align: 1px;
  white-space: nowrap;
}
.agent-ws-session-dot.st-running {
  color: #2e6fd0;
  background: rgba(46, 111, 208, 0.14);
}
.agent-ws-session-dot.st-pending {
  color: #b0882e;
  background: rgba(214, 158, 46, 0.16);
}
/* 「已恢复」标记：磁盘发现补入的会话（localStorage 索引曾丢失） */
.agent-ws-session-badge {
  display: inline-block;
  margin-left: 4px;
  padding: 0 4px;
  font-size: 9px;
  line-height: 14px;
  vertical-align: 1px;
  color: #8a6d1f;
  background: rgba(214, 158, 46, 0.16);
  border-radius: 3px;
  white-space: nowrap;
}
.agent-ws-session-time {
  font-size: 10px;
  opacity: 0.5;
}
.agent-ws-session-del {
  position: absolute;
  top: 6px;
  right: 6px;
  width: 16px;
  height: 16px;
  display: none;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 50%;
  padding: 0;
}
.agent-ws-session-item:hover .agent-ws-session-del {
  display: flex;
}
.agent-ws-session-del:hover {
  color: #d64545;
  background: var(--bg-hover);
}
.agent-ws-session-del svg {
  width: 10px;
  height: 10px;
}
.agent-ws-session-empty {
  padding: 12px 8px;
  font-size: 12px;
  opacity: 0.5;
  text-align: center;
}
.agent-ws-session-clear {
  margin: 2px 8px 10px;
  padding: 6px 8px;
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  flex-shrink: 0;
}
.agent-ws-session-clear:hover {
  color: #d64545;
  border-color: rgba(214, 69, 69, 0.4);
  background: var(--bg-hover);
}
.agent-ws-session-clear svg {
  width: 10px;
  height: 10px;
}

/* 聊天区（position: relative 供运行历史浮层 .agent-ws-history 定位） */
.agent-ws-chat {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
}

/* 状态横幅 */
.agent-ws-banner {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px;
  font-size: 12px;
  min-height: 38px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-subtle);
}
/* 横幅按钮固定高度：避免中文/ASCII 字体行盒差异导致展开/折叠时条高变化 */
.agent-ws-banner .btn-sm {
  height: 24px;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  white-space: nowrap;
}
.agent-ws-banner-status {
  font-weight: 600;
}
.agent-ws-banner-error {
  color: #d64545;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 240px;
  flex-shrink: 1;
}
.agent-ws-banner-queue {
  color: #b0882e;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 260px;
  flex-shrink: 1;
}
.agent-ws-banner.status-running .agent-ws-banner-status { color: #2e6fd0; }
.agent-ws-banner.status-pending .agent-ws-banner-status { color: #2e6fd0; }
.agent-ws-banner.status-success .agent-ws-banner-status { color: #2e9e5b; }
.agent-ws-banner.status-failed .agent-ws-banner-status { color: #d64545; }
.agent-ws-banner.status-timeout .agent-ws-banner-status { color: #d08a2e; }
.agent-ws-banner.status-cancelled .agent-ws-banner-status { color: #8a8a8a; }
.agent-ws-banner-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  opacity: 0.75;
}
.agent-ws-banner-actions {
  display: flex;
  gap: 6px;
}
.agent-ws-banner-toggle {
  opacity: 0.6;
}
.agent-ws-banner-toggle:hover {
  opacity: 1;
}
/* 排队提示可点击（被其他会话占用时）：虚线强调 + hover 变 accent */
.agent-ws-banner-queue.clickable {
  cursor: pointer;
  text-decoration: underline;
  text-decoration-style: dotted;
  text-underline-offset: 2px;
}
.agent-ws-banner-queue.clickable:hover {
  color: #2e6fd0;
}
.agent-ws-banner-actions .btn-sm.active {
  background: rgba(46, 111, 208, 0.12);
  border-color: rgba(46, 111, 208, 0.35);
  color: #2e6fd0;
}

/* 上下文水位条：消息数 / 估算 token；接近上限时置顶提示开新会话 */
.agent-ws-usage {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 14px;
  font-size: 11px;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
  background: var(--bg-subtle);
}
.agent-ws-usage.warn {
  color: #b0882e;
  background: rgba(214, 158, 46, 0.08);
}
.agent-ws-usage-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-usage-new {
  flex-shrink: 0;
  height: 20px;
  padding: 0 8px;
  font-size: 11px;
}

/* 运行历史面板：覆盖整个聊天区的浮层（顶部含标题与关闭；依托 .agent-ws-chat 的 relative 定位） */
.agent-ws-history {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 12;
  background: var(--bg);
  display: flex;
  flex-direction: column;
}
.agent-ws-history-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.agent-ws-history-title {
  font-size: 12px;
  font-weight: 600;
}
.agent-ws-history-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  background: none;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 5px;
}
.agent-ws-history-close:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.agent-ws-history-close svg {
  width: 12px;
  height: 12px;
}
.agent-ws-history-list {
  list-style: none;
  margin: 0;
  padding: 8px 10px;
  overflow-y: auto;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.agent-ws-history-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-subtle);
}
.agent-ws-history-status {
  font-size: 11px;
  font-weight: 600;
  flex-shrink: 0;
  min-width: 34px;
}
.agent-ws-history-status.st-running,
.agent-ws-history-status.st-pending {
  color: #2e6fd0;
}
.agent-ws-history-status.st-success {
  color: #2e9e5b;
}
.agent-ws-history-status.st-failed {
  color: #d64545;
}
.agent-ws-history-status.st-timeout {
  color: #d08a2e;
}
.agent-ws-history-status.st-cancelled {
  color: #8a8a8a;
}
.agent-ws-history-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.agent-ws-history-instr {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-history-meta {
  font-size: 10px;
  opacity: 0.6;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--mono-font, monospace);
}
.agent-ws-history-actions {
  flex-shrink: 0;
}
.agent-ws-history-actions .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
.agent-ws-history-empty {
  padding: 20px;
  text-align: center;
  font-size: 12px;
  opacity: 0.5;
}
.agent-ws-banner-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid rgba(46, 111, 208, 0.25);
  border-top-color: #2e6fd0;
  border-radius: 50%;
  animation: agent-ws-spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes agent-ws-spin {
  to { transform: rotate(360deg); }
}

/* 消息区 */
.agent-ws-messages {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.agent-ws-hint {
  text-align: center;
  opacity: 0.6;
  font-size: 13px;
  padding: 24px 0;
}
.agent-ws-hint-empty {
  white-space: pre-line;
  line-height: 1.8;
}
.agent-ws-msg-row {
  display: flex;
  flex-direction: column;
}
.agent-ws-msg-row.role-user {
  align-items: flex-end;
}
.agent-ws-msg-row.role-assistant {
  align-items: flex-start;
}

/* 气泡 */
.agent-ws-bubble {
  max-width: 86%;
  border-radius: 10px;
  padding: 9px 12px;
  font-size: 13px;
  line-height: 1.55;
}
.agent-ws-bubble-user {
  background: rgba(46, 111, 208, 0.13);
  border: 1px solid rgba(46, 111, 208, 0.28);
  border-top-right-radius: 3px;
  color: var(--text);
}
.agent-ws-bubble-assistant {
  /* 覆盖 .agent-ws-bubble 的 max-width:86%——右侧留白由 margin-right 保证与左侧 padding(14px) 一致 */
  max-width: none;
  margin-right: 14px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-top-left-radius: 3px;
}
.agent-ws-bubble-model {
  font-size: 10px;
  opacity: 0.5;
  margin-bottom: 4px;
  font-family: var(--mono-font, monospace);
}
.agent-ws-bubble-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: 5px;
}
.agent-ws-msg-text {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}
/* 非成功终态内联备注 + 重试入口（挂在对应 user 气泡下） */
.agent-ws-run-failed {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 7px;
  padding: 5px 8px;
  border-radius: 6px;
  border: 1px solid rgba(214, 69, 69, 0.35);
  background: rgba(214, 69, 69, 0.08);
  font-size: 11px;
  line-height: 1.45;
  max-width: 100%;
}
/* 被取消不是错误（用户主动停 / 应用重启清理），用中性色，不伪装成报错 */
.agent-ws-run-failed.run-cancelled {
  border-color: var(--border);
  background: var(--bg-subtle);
}
.agent-ws-run-failed-status {
  font-weight: 600;
  color: #d64545;
  flex-shrink: 0;
}
.agent-ws-run-failed.run-cancelled .agent-ws-run-failed-status {
  color: var(--text-muted);
}
.agent-ws-run-failed-text {
  color: var(--text-muted);
  word-break: break-word;
  flex: 1;
  min-width: 60px;
}
/* 重试操作：与提示同行，空间不足时换行 */
.agent-ws-run-failed-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}
.agent-ws-run-failed-actions .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
/* 超时引导（评审 3.6）：行动建议独占一行 + 就地调时长 */
.agent-ws-run-advice {
  flex-basis: 100%;
  color: var(--text-muted);
  word-break: break-word;
}
.agent-ws-run-advice-actions {
  flex-shrink: 0;
}
.agent-ws-run-advice-actions .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
.agent-ws-run-advice-adjust {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
  align-items: center;
}
.agent-ws-run-advice-adjust .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
.agent-ws-timeout-input {
  width: 72px;
  padding: 2px 6px;
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  color: var(--text);
}
.agent-ws-timeout-input:focus {
  outline: none;
  border-color: var(--accent, #2e6fd0);
}

/* 折叠块（思考 / 工具详情） */
.agent-ws-fold {
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--bg-subtle);
  margin: 6px 0;
  font-size: 12px;
}
.agent-ws-fold summary {
  padding: 5px 9px;
  cursor: pointer;
  opacity: 0.75;
  font-size: 11px;
  user-select: none;
}
.agent-ws-fold-thinking summary {
  color: #8a6d3b;
}
.agent-ws-fold-prompt summary {
  color: var(--text-muted);
}
.agent-ws-fold-body {
  margin: 0;
  padding: 8px 10px;
  border-top: 1px solid var(--border);
  font-family: var(--mono-font, monospace);
  font-size: 11px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 260px;
  overflow-y: auto;
  background: var(--bg);
  border-radius: 0 0 7px 7px;
}

/* 工具卡片（toolCall / toolResult / bash） */
.agent-ws-tool-card {
  /* 右侧留白与消息区左侧 padding(14px) 一致，与 assistant 气泡同步 */
  margin-right: 14px;
  align-self: flex-start;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-subtle);
  font-size: 12px;
  overflow: hidden;
}
.agent-ws-tool-card.tool-error {
  border-color: rgba(214, 69, 69, 0.5);
}
.agent-ws-tool-head {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 6px 10px;
}
.agent-ws-tool-icon {
  width: 13px;
  height: 13px;
  color: var(--accent, #2e6fd0);
  flex-shrink: 0;
}
.agent-ws-tool-name {
  font-family: var(--mono-font, monospace);
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-tool-tag {
  margin-left: auto;
  font-size: 10px;
  opacity: 0.55;
  flex-shrink: 0;
}
.agent-ws-tool-card details summary {
  padding: 5px 10px;
  border-top: 1px solid var(--border);
  cursor: pointer;
  font-size: 11px;
  opacity: 0.7;
  user-select: none;
}

/* chips / 徽章 */
.agent-ws-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--bg-subtle);
  white-space: nowrap;
  /* 截断限制移到 .agent-ws-chip-text——之前 max-width+overflow 会把删除按钮一起裁掉 */
}
.agent-ws-chip-text {
  max-width: 170px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 引用 chip 全文悬浮提示（fixed 避免被消息区滚动容器裁剪） */
.agent-ws-chip-tooltip {
  position: fixed;
  z-index: 10002;
  max-width: min(480px, calc(100vw - 32px));
  padding: 9px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 8px);
  box-shadow: var(--shadow-lg, 0 4px 16px rgba(0, 0, 0, 0.25));
  color: var(--text);
  font-size: 12px;
  line-height: 1.55;
  overflow-wrap: anywhere;
  pointer-events: none;
}
.agent-ws-chip-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 50%;
  flex-shrink: 0;
}
.agent-ws-chip-remove:hover {
  color: #d64545;
  background: var(--bg-hover);
}
.agent-ws-chip-remove svg {
  width: 9px;
  height: 9px;
}
.agent-ws-skill-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  font-size: 11px;
  border-radius: 999px;
  background: rgba(46, 111, 208, 0.12);
  color: #2e6fd0;
  max-width: 220px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 输入区 */
.agent-ws-input {
  position: relative;
  border-top: 1px solid var(--border);
  padding: 10px 14px 12px;
  background: var(--bg);
}
.agent-ws-input-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: 6px;
  min-height: 18px;
}
.agent-ws-input-row {
  display: flex;
  gap: 8px;
  align-items: flex-end;
}
.agent-ws-textarea {
  flex: 1;
  resize: none;
  padding: 8px 10px;
  font-size: 13px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  line-height: 1.5;
  font-family: inherit;
}
.agent-ws-textarea:focus {
  outline: none;
  border-color: var(--accent, #2e6fd0);
}
.agent-ws-submit {
  flex-shrink: 0;
  height: 36px;
}
.agent-ws-submit.agent-ws-stop {
  background: #d64545;
  border-color: #d64545;
}
.agent-ws-submit.agent-ws-stop:hover {
  background: #c0392b;
  border-color: #c0392b;
}

/* 底部操作行：模型选择（左）+ 发送/停止（右），共占一行 */
.agent-ws-input-actions {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 6px;
}
.agent-ws-model-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  padding: 3px 9px;
  font-size: 11px;
  color: var(--text);
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.12s ease, background 0.12s ease;
}
.agent-ws-model-btn:hover,
.agent-ws-model-btn.open {
  border-color: var(--accent, #2e6fd0);
  background: rgba(46, 111, 208, 0.08);
}
.agent-ws-model-icon {
  width: 12px;
  height: 12px;
  color: var(--accent, #2e6fd0);
  flex-shrink: 0;
}
.agent-ws-model-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-model-caret {
  width: 11px;
  height: 11px;
  color: var(--text-muted);
  flex-shrink: 0;
  transition: transform 0.12s ease;
}
.agent-ws-model-btn.open .agent-ws-model-caret {
  transform: rotate(180deg);
}
/* 模型选择菜单：作为 .agent-ws-input-actions 的子元素定位（position:relative 的 parent），
   bottom:100% 使面板紧贴在操作行（模型按钮）上方弹出，而非输入区顶部；
   高优先级选择器覆盖 .agent-ws-menu 基类的 footer 定位，不依赖顺序 */
.agent-ws-menu.agent-ws-menu-model {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  right: auto;
  top: auto;
  width: min(340px, 100%);
}

/* 引用菜单 */
.agent-ws-menu {
  position: absolute;
  bottom: calc(100% - 8px);
  left: 14px;
  right: 14px;
  max-height: 260px;
  overflow-y: auto;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: var(--shadow-lg);
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  z-index: 20;
}
.agent-ws-menu-title {
  font-size: 11px;
  font-weight: 600;
  opacity: 0.6;
  padding: 2px 8px 4px;
}
.agent-ws-menu-group {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  font-weight: 600;
  opacity: 0.5;
  padding: 6px 8px 2px;
}
.agent-ws-menu-group-icon {
  width: 12px;
  height: 12px;
}
.agent-ws-menu-item {
  display: flex;
  flex-direction: column;
  gap: 1px;
  text-align: left;
  padding: 6px 8px;
  font-size: 12px;
  border: none;
  background: none;
  color: var(--text);
  cursor: pointer;
  border-radius: 6px;
}
.agent-ws-menu-item:hover,
.agent-ws-menu-item.selected {
  background: var(--bg-hover);
}
.agent-ws-menu-main {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-menu-sub {
  font-size: 10px;
  opacity: 0.55;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--mono-font, monospace);
}
.agent-ws-menu-empty {
  padding: 10px 8px;
  font-size: 12px;
  opacity: 0.6;
  text-align: center;
}
</style>
