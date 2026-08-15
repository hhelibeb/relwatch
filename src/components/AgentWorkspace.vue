<script setup lang="ts">
import { ref, computed, inject, onMounted, onUnmounted, nextTick, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import MarkdownContent from './common/MarkdownContent.vue'
import { events } from '../bindings'
import { ShowToastKey, type AgentEntityRefSeed, type AgentWorkspaceSeed } from '../injection-keys'
import {
  getAgentConfig,
  runAgentJob,
  listAgentRuns,
  listAgentMessages,
  deleteAgentSession,
  cancelAgentRun,
  openAgentSession,
  getAgentSessionCommand,
  type AgentChatMessage,
  type AgentRun,
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
}
const SESSIONS_STORAGE_KEY = 'relwatch.agent.sessions.v1'
// 会话侧栏折叠状态（默认折叠，聊天区全宽；localStorage 持久化）
const SIDEBAR_STORAGE_KEY = 'relwatch.agent.sidebar.v1'
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
  localStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify(sessions.value.slice(0, 30)))
}

const sessions = ref<SessionMeta[]>(loadSessions())
// 激活会话：最近一个优先；无历史则新建
const activeKey = ref(sessions.value[0]?.key ?? newSessionKey())
const sessionTitle = computed(() => {
  const meta = sessions.value.find((s) => s.key === activeKey.value)
  return meta ? meta.title : t('agent.session_new')
})
const isNewSession = computed(() => !sessions.value.some((s) => s.key === activeKey.value))

function newSessionKey(): string {
  return crypto.randomUUID()
}

function switchSession(key: string) {
  if (key === activeKey.value) return
  // 当前会话有运行中的 run：先中止（RPC abort），避免切换会话打断生成
  if (activeRunId.value !== null) {
    void cancelAgentRun(activeRunId.value).catch(() => {})
  }
  activeKey.value = key
  stopPolling()
  activeRunId.value = null
  cancelling.value = false
  liveMessages.value = []
  historySnapshot.value = []
  void loadChat()
  entities.value = []
  skillPath.value = null
  instruction.value = ''
}

function startNewSession() {
  if (isNewSession.value && messages.value.length === 0 && runs.value.length === 0) return
  activeKey.value = newSessionKey()
  entities.value = []
  skillPath.value = null
  instruction.value = ''
  messages.value = []
  runs.value = []
  liveMessages.value = []
  historySnapshot.value = []
  activeRunId.value = null
  cancelling.value = false
  void loadChat()
  nextTick(() => textareaRef.value?.focus())
}

async function handleDeleteSession(key: string) {
  try {
    await deleteAgentSession(key)
    const idx = sessions.value.findIndex((s) => s.key === key)
    if (idx >= 0) sessions.value.splice(idx, 1)
    persistSessions()
    if (key === activeKey.value) {
      activeKey.value = sessions.value[0]?.key ?? newSessionKey()
      entities.value = []
      skillPath.value = null
      instruction.value = ''
      await loadChat()
    }
    showToast(t('agent.session_deleted'))
  } catch (e) {
    showToast(String(e))
  }
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
const runs = ref<AgentRun[]>([])
const textareaRef = ref<HTMLTextAreaElement | null>(null)
const scrollRef = ref<HTMLElement | null>(null)
// 当前会话正在运行的 run_id（提交后设置，终态事件后清空；用于「停止」）
const activeRunId = ref<number | null>(null)
// 是否处于可停止状态：提交后、且对应 run 尚未终态
const canStop = computed(() => {
  if (activeRunId.value === null) return false
  const run = runs.value.find((r) => r.id === activeRunId.value)
  if (!run) return true // 提交后 runs 尚未刷新，视为运行中
  return run.status === 'pending' || run.status === 'running'
})
const cancelling = ref(false)

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
  const run = runs.value.find((r) => r.id === payload.run_id)
  if (run) activeRunId.value = payload.run_id

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

async function loadChat() {
  messagesLoading.value = true
  try {
    await Promise.all([loadRuns(), loadMessages()])
  } finally {
    messagesLoading.value = false
    scrollToBottom()
  }
}

async function loadCatalog() {
  try {
    const [cfg, srcs, rels] = await Promise.all([getAgentConfig(), listSources(), getReleases()])
    skills.value = cfg.skills
    sources.value = srcs
    releases.value = rels
  } catch {
    // 目录加载失败不阻塞工作区使用（名称映射降级为 #id）
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
    await loadMessages()
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
    })
    activeRunId.value = runId
    track('agent.submit')
    instruction.value = ''
    // 会话登记（标题取首次指令前 40 字）
    const now = Date.now()
    const idx = sessions.value.findIndex((s) => s.key === activeKey.value)
    const title = cleaned.trim() ? [...cleaned.trim()].slice(0, 40).join('') : sessionTitle.value
    if (idx >= 0) {
      sessions.value[idx] = { ...sessions.value[idx], title, updatedAt: now }
    } else {
      sessions.value.unshift({ key: activeKey.value, title, updatedAt: now })
    }
    persistSessions()
    await loadChat()
    startPolling()
  } catch (e) {
    showToast(String(e))
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
// runs（倒序）按顺序对位 user 消息（时间窗校验，防对位错乱）
const userRunMap = computed<Map<number, AgentRun>>(() => {
  const map = new Map<number, AgentRun>()
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

function runForMessage(idx: number): AgentRun | undefined {
  return userRunMap.value.get(idx)
}

/** 最近一次 run（状态横幅用）。 */
const latestRun = computed<AgentRun | undefined>(() => runs.value[0])

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
async function handleOpenSession(run: AgentRun) {
  if (!run.session_path) return
  try {
    await openAgentSession(run.id)
  } catch (e) {
    showToast(String(e))
  }
}

async function handleCopySessionCommand(run: AgentRun) {
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

function pickEntity(kind: 'source' | 'release', id: number) {
  replaceTrigger(`[[${kind}:${id}]] `)
  showEntityMenu.value = false
}

// ── 键盘统一入口 ──
// 菜单打开时：按键先交给菜单（Enter/Tab 选择项、Escape 关闭），一律不触发提交；
// 无菜单时：无修饰键 Enter 提交。避免「选菜单项的同时消息被自动发出」。
function handleKeydown(e: KeyboardEvent) {
  if (showSkillMenu.value || showEntityMenu.value) {
    handleMenuKeydown(e)
    return
  }
  if (e.key === 'Enter' && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
    e.preventDefault()
    void handleSubmit()
  }
}

// ── 菜单键盘导航 ──
function handleMenuKeydown(e: KeyboardEvent) {
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
async function onRunFinished(payload: { run_id: number; session_key: string; status: string }) {
  if (payload.session_key !== activeKey.value) return
  stopPolling()
  activeRunId.value = null
  cancelling.value = false
  // 兜底清理：正常路径 agent_settled 已清；abort / 超时 / 模型错误等
  // 场景下 agent_settled 可能不达，run 终态事件统一收尾
  liveMessages.value = []
  historySnapshot.value = []
  await loadChat()
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
  unlistenRunFinished = await listen<{ run_id: number; session_key: string; status: string }>('agent-run-finished', (e) => {
    void onRunFinished(e.payload)
  })
  unlistenRpcStream = await events.agentRpcStream.listen((e) => {
    handleRpcStream(e.payload)
  })
  await nextTick()
  textareaRef.value?.focus()
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
          <span class="agent-ws-banner-text">{{ latestRun.instruction || sessionTitle }}</span>
          <span v-if="latestRun.status === 'running' || latestRun.status === 'pending'" class="agent-ws-banner-spinner" aria-hidden="true"></span>
          <span v-if="latestRun.session_path" class="agent-ws-banner-actions">
            <button class="btn-sm" :title="t('agent.open_session')" @click="handleOpenSession(latestRun)">{{ t('agent.open_session') }}</button>
            <button class="btn-sm" :title="t('agent.copy_command_hint')" @click="handleCopySessionCommand(latestRun)">{{ t('agent.copy_command') }}</button>
          </span>
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
                    {{ entityKindLabel(e.kind) }} · {{ entityLabel(e) }}
                  </span>
                  <span v-if="runForMessage(idx)?.skill_path" class="agent-ws-skill-badge">@{{ skillShortName(runForMessage(idx)!.skill_path ?? '') }}</span>
                </div>
                <p class="agent-ws-msg-text">{{ stripUserInstructionWrapper(stripSkillBlock(blockText(msg.blocks))) || '…' }}</p>
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
              {{ entityKindLabel(e.kind) }} · {{ entityLabel(e) }}
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
            <button
              class="btn-primary agent-ws-submit"
              :class="{ 'agent-ws-stop': canStop }"
              :disabled="submitting && !canStop"
              :title="canStop ? t('agent.stop_hint') : ''"
              @click="canStop ? handleCancel() : handleSubmit()"
            >
              {{ canStop ? (cancelling ? t('agent.stopping') : t('agent.stop')) : submitting ? t('agent.running') : t('agent.submit') }}
            </button>
          </div>
          <p class="agent-ws-input-hint">{{ t('agent.input_hint') }}</p>

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
            v-for="s in sessions"
            :key="s.key"
            class="agent-ws-session-item"
            :class="{ active: s.key === activeKey }"
            :title="s.title"
            @click="switchSession(s.key)"
          >
            <span class="agent-ws-session-name">{{ s.title }}</span>
            <span class="agent-ws-session-time">{{ formatDate(new Date(s.updatedAt).toISOString()) }}</span>
            <button class="agent-ws-session-del" :title="t('agent.delete_session')" @click.stop="handleDeleteSession(s.key)">
              <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
            </button>
          </li>
          <li v-if="isNewSession" class="agent-ws-session-item active agent-ws-session-item-new">
            <span class="agent-ws-session-name">{{ t('agent.session_new') }}</span>
          </li>
          <li v-if="sessions.length === 0 && !isNewSession" class="agent-ws-session-empty">{{ t('agent.session_empty') }}</li>
        </ul>
      </aside>
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

/** 剥离整条被 <用户指令> 标签包裹的消息外层标签（多轮精简消息的显示美化）。
 * 仅当标签完整包裹整条消息（开头 <用户指令>、结尾 </用户指令>）时剥离；
 * 标签位于消息中间时（如首轮完整模板）保留原样，保证完整上下文可见。 */
function stripUserInstructionWrapper(text: string): string {
  return text.replace(/^\s*<用户指令>\s*([\s\S]*?)\s*<\/用户指令>\s*$/, '$1').trim()
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

function runEntities(run: AgentRun | undefined): AgentEntityRefSeed[] {
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
  border-left: 1px solid var(--border);
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

/* 聊天区 */
.agent-ws-chat {
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
  border-bottom: 1px solid var(--border);
  background: var(--bg-subtle);
}
.agent-ws-banner-status {
  font-weight: 600;
}
.agent-ws-banner.status-running .agent-ws-banner-status { color: #2e6fd0; }
.agent-ws-banner.status-pending .agent-ws-banner-status { color: #2e6fd0; }
.agent-ws-banner.status-success .agent-ws-banner-status { color: #2e9e5b; }
.agent-ws-banner.status-failed .agent-ws-banner-status { color: #d64545; }
.agent-ws-banner.status-timeout .agent-ws-banner-status { color: #d08a2e; }
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
  max-width: 86%;
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
  max-width: 220px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
.agent-ws-input-hint {
  margin: 6px 0 0;
  font-size: 11px;
  opacity: 0.55;
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
