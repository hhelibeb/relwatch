// ── 聊天核心（B 域：历史加载 / RPC 流事件合帧 / 轮询 / 快照 / 滚动；
//    C 域：提交 / 停止 / 重试）──
// 跨域依赖全部经入参以 ref/回调注入（§4.2），不 import 其他域的模块：
// - 提交输入（F 域 composer）：instruction / entities / skillPath / files（读 + 成功后清空）
// - 模型（D 域）：effectiveModel（提交生效模型）、oneShotModel / modelOnce（提交后消费清空）、
//   selectedModel（重试回填 run 的模型）
// - 用量（H 域）：usage / loadUsage（loadChat 预清 + 联动刷新，避免切换时闪现旧水位）
// - 会话域：sessionTitle / persistSessionMeta 经编排层接线
// - 进程指示灯（E 域）：loadRpcStatus（轮询启动 / run 收尾时顺带刷新）
import { computed, nextTick, onUnmounted, ref, watch, type Ref } from 'vue'
import {
  cancelAgentRun,
  getAgentQueue,
  getAgentQueueStatus,
  listAgentMessages,
  listAgentRuns,
  runAgentJob,
  type AgentChatMessage,
  type AgentModelRef,
  type AgentQueueItem,
  type AgentQueueStatus,
  type AgentRunSummary,
  type AgentSessionUsage,
} from '../../api/agent'
import type { Source } from '../../api/sources'
import type { ReleaseInfo } from '../../api/releases'
import type { AgentEntityRefSeed } from '../../injection-keys'
import { t } from '../../i18n'
import { skillShortName } from '../../utils'
import { track } from '../../composables/useUsageTracking'
import type { SessionSwitchMode } from './useAgentSessions'
import {
  escapeRegExp,
  runEntities,
  runErrorText,
  runFiles,
  runModel,
  splitUserBlocks,
  timeAdjacent,
} from './agentChatUtils'

export function useAgentChat(deps: {
  activeKey: Ref<string>
  showToast: (message: string) => void
  // ── 提交输入（F 域）──
  instruction: Ref<string>
  entities: Ref<AgentEntityRefSeed[]>
  skillPath: Ref<string | null>
  files: Ref<string[]>
  focusAtEnd: () => void
  // ── 模型（D 域）──
  effectiveModel: Ref<AgentModelRef | null>
  selectedModel: Ref<AgentModelRef | null>
  oneShotModel: Ref<AgentModelRef | null>
  modelOnce: Ref<boolean>
  // ── 用量（H 域）──
  usage: Ref<AgentSessionUsage | null>
  loadUsage: () => Promise<void>
  // ── 会话域（经编排层接线）──
  sessionTitle: Ref<string>
  persistSessionMeta: (key: string, title: string, model: AgentModelRef | null) => void
  // ── 菜单互斥：菜单打开时 Enter 不提交 ──
  showSkillMenu: Ref<boolean>
  showEntityMenu: Ref<boolean>
  // ── 进程指示灯（E 域）──
  loadRpcStatus: () => Promise<void>
  // ── 实体目录（重试时校验引用实体仍然存在）──
  sources: Ref<Source[]>
  releases: Ref<ReleaseInfo[]>
  // ── 全局队列（编排层持有：侧栏状态点 / 横幅共用，本域 loadQueue 写入）──
  queueActive: Ref<AgentQueueItem[]>
}) {
  const {
    activeKey,
    showToast,
    instruction,
    entities,
    skillPath,
    files,
    focusAtEnd,
    effectiveModel,
    selectedModel,
    oneShotModel,
    modelOnce,
    usage,
    loadUsage,
    sessionTitle,
    persistSessionMeta,
    showSkillMenu,
    showEntityMenu,
    loadRpcStatus,
    sources,
    releases,
    queueActive,
  } = deps

  // ── 聊天状态 ──
  const messages = ref<AgentChatMessage[]>([])
  const messagesLoading = ref(false)
  const submitting = ref(false)
  const runs = ref<AgentRunSummary[]>([])
  const scrollRef = ref<HTMLElement | null>(null)

  // ── 全局队列状态（「排队中」提示：其他会话占用 + 队列位置）──
  const queueInfo = ref<AgentQueueStatus | null>(null)

  async function loadQueueInfo() {
    try {
      queueInfo.value = await getAgentQueueStatus(activeKey.value)
    } catch {
      queueInfo.value = null
    }
  }

  async function loadQueue() {
    try {
      queueActive.value = await getAgentQueue()
    } catch {
      queueActive.value = []
    }
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

  /** 该消息是否属于流式实时内容（仍在增长的 live 消息）：流式 Markdown 走
   *  noCache 渲染——内容每个合帧批次都变，写缓存只会以内容前缀形式冲刷掉
   *  列表/详情等静态场景的缓存条目（FIFO 100 条撑不过一次长输出的 delta 数）。 */
  function isLiveMessage(msg: AgentChatMessage): boolean {
    return liveMessages.value.indexOf(msg) >= 0
  }

  /** 当前流式 assistant 消息（没有则创建一条）。 */
  function liveAssistant(): AgentChatMessage {
    let cur = liveMessages.value[liveMessages.value.length - 1]
    if (!cur || cur.role !== 'assistant') {
      cur = { role: 'assistant', blocks: [], timestamp: new Date().toISOString(), model: null }
      liveMessages.value.push(cur)
    }
    return cur
  }

  /** 处理单个 pi RPC 流事件（打字机文本 / 工具状态 / 流式 bash 输出）。
   *  事件原序逐个处理，处理逻辑与合帧前完全一致；事件含 session_key，
   *  flush 时逐个重新校验，切会话后旧事件不会写入新会话的流式消息。 */
  function processRpcEvent(payload: { session_key: string; run_id: number; event: string }) {
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

  // ── 流式事件合帧：pi 的 delta 事件可达每秒数十上百条，逐条触发渲染会让主线程
  // 饱和（每 delta 一次全量 Markdown 重解析 + 整组件重渲染）。事件先入队，
  // 50ms 窗口内合并成一批处理，渲染次数从 delta 频率降到 ≤20 次/秒。
  // 处理逻辑零改动（processRpcEvent = 原逐条处理函数），批末统一跟随滚动
  // （替代原 deep watch —— 原地追加 text 只有深度遍历才能监听到，代价是每个
  // delta 遍历整棵流式消息树；现在每批只滚一次）。
  const RPC_FLUSH_MS = 50
  let pendingRpcEvents: { session_key: string; run_id: number; event: string }[] = []
  let rpcFlushTimer: ReturnType<typeof setTimeout> | undefined

  /** 丢弃未处理的流式事件（切会话 / run 终态 / 卸载时调用）：
   *  终态后 UI 以 loadChat 全量校准为准，残留的流式 delta 已无意义；
   *  特别是 run 终态（onRunFinished）可能先于队列 flush 到达，不清队列
   *  会让旧 delta 在清空 liveMessages 后又重建出幽灵流式消息。 */
  function discardPendingRpcEvents() {
    if (rpcFlushTimer !== undefined) {
      clearTimeout(rpcFlushTimer)
      rpcFlushTimer = undefined
    }
    pendingRpcEvents = []
  }

  /** 事件监听回调：入队并调度合帧 flush。 */
  function handleRpcStream(payload: { session_key: string; run_id: number; event: string }) {
    pendingRpcEvents.push(payload)
    if (rpcFlushTimer === undefined) {
      rpcFlushTimer = setTimeout(flushRpcEvents, RPC_FLUSH_MS)
    }
  }

  /** 合帧 flush：按到达顺序处理整批事件（单个事件异常不中断后续），批末跟随滚动。 */
  function flushRpcEvents() {
    rpcFlushTimer = undefined
    const batch = pendingRpcEvents
    pendingRpcEvents = []
    for (const payload of batch) {
      try {
        processRpcEvent(payload)
      } catch {
        // 单事件处理异常不丢弃同批后续事件（原逐条处理下异常只影响该事件）
      }
    }
    scrollToBottomIfNear()
  }

  // ── 轮询：run 进行期间的兜底刷新（RPC 流事件丢帧 / agent_settled 不达时 UI 仍能收敛）──
  let pollTimer: ReturnType<typeof setInterval> | undefined
  let pollDeadline: number | undefined

  function startPolling() {
    stopPolling()
    pollDeadline = Date.now() + 120_000 // 兜底：120s 后强制停止
    pollTimer = setInterval(async () => {
      if (pollDeadline && Date.now() > pollDeadline) {
        stopPolling()
        return
      }
      // 全量消息拉取只在流式内容尚未接管显示时进行（切回正在运行的会话、
      // delta 首达前，它是 pi 已落盘进度的唯一可见通道）；流式接管后
      // displayedMessages 走快照+流式，全量读盘结果不进展示链，跳过以省下
      // MB 级读盘 + 全量 JSONL 解析 + IPC 序列化。队列状态是轻量 SQL，
      // 保留刷新（侧栏状态点 / 排队横幅）。
      const tasks: Promise<void>[] = [loadQueue()]
      if (liveMessages.value.length === 0) tasks.push(loadMessages())
      await Promise.all(tasks)
      scrollToBottomIfNear()
    }, 1500)
    // 进程指示灯：轮询周期内顺带刷新（run 开始/结束时进程状态会变，重启标记也会被消费）
    void loadRpcStatus()
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
    // 只附加了文件、没写指令也算有效提交（"看看这个日志" 的意图已由附件承载）
    if (cleaned.trim() === '' && merged.length === 0 && files.value.length === 0) {
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
        model: effectiveModel.value,
        files: files.value.length > 0 ? [...files.value] : null,
      })
      submittedRunId.value = runId
      track('agent.submit')
      instruction.value = ''
      // 单次覆盖已消费：清掉一次性选择与附件，回落会话默认（不写 SessionMeta）
      oneShotModel.value = null
      modelOnce.value = false
      files.value = []
      // 会话登记（标题取首次指令前 40 字）+ 固化本次模型选择 + 清除草稿标记
      // （新建即登记后 key 恒在索引中；draft 清除 = 已提交，不再是「新会话」）
      // 注意固化的是 selectedModel（会话长期选择），一次性覆盖不落库。
      const title = cleaned.trim() ? [...cleaned.trim()].slice(0, 40).join('') : sessionTitle.value
      persistSessionMeta(activeKey.value, title, selectedModel.value)
      await loadChat()
      startPolling()
    } catch (e) {
      showToast(String(e))
      // 提交被拒（未创建 run，不会有 AgentRunFinished 事件兜底）：
      // 清掉本地回显与历史快照，避免失败消息永久滞留成「幽灵消息」
      discardPendingRpcEvents()
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

  // ── 事件 ──
  async function onRunFinished() {
    // 不按 session_key 过滤：当前会话若有活跃 run（pending/running），任意会话的
    // run 结束都可能影响它（其他会话结束 → 本会话排队 run 开始执行；本会话结束 →
    // 横幅/停止按钮收尾）。统一刷新，保证「排队中」横幅在别的会话结束后自动更新。
    if (activeRunId.value === null) return
    stopPolling()
    cancelling.value = false
    // 兜底清理：正常路径 agent_settled 已清；abort / 超时 / 模型错误等
    // 场景下 agent_settled 可能不达，run 终态事件统一收尾。
    // 同时丢弃合帧队列：终态可能先于队列 flush 到达，残留 delta 会在
    // 清空后重建出幽灵流式消息（loadChat 全量校准才是权威）。
    discardPendingRpcEvents()
    liveMessages.value = []
    historySnapshot.value = []
    // run 收尾：进程可能刚重启过、推迟标记也刚被消费，指示灯需同步
    await Promise.all([loadChat(), loadRpcStatus()])
  }

  // ── 消息渲染辅助 ──
  // 时间窗对位（fallback）：runs（倒序）按顺序对位 user 消息。messageDecorations 优先用
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

  /** user 消息渲染装饰（预计算）：run 对位、引用实体、主/折叠文本拆分。
   *  流式期间渲染函数每批重跑，模板内联函数（runs.find + Date 解析 + JSON.parse
   *  + 正则拆分）会在每条 user 消息上重复十余次——并入 computed 每批每条只算一次
   *  （同侧栏 sessionsWithState 的预计算先例）。对位口径与原内联函数完全一致：
   *  run_id 直连（60s 邻近校验）→ 时间窗兜底。 */
  const messageDecorations = computed(() => {
    const runById = new Map(runs.value.map((r) => [r.id, r]))
    const fallback = userRunMap.value
    return displayedMessages.value.map((msg, idx) => {
      if (msg.role !== 'user') return null
      // run_id 直连（后端 list_agent_messages 已按创建顺序对位；直连命中后仍
      // 二次校验时间邻近，防御旧版本绑定/后端异常），不通过落回时间窗兜底
      let run: AgentRunSummary | undefined
      if (msg.run_id) {
        const byId = runById.get(msg.run_id)
        if (byId && byId.started_at && timeAdjacent(byId.started_at, msg.timestamp)) run = byId
      }
      if (!run) run = fallback.get(idx)
      const split = splitUserBlocks(msg.blocks)
      return {
        run,
        entities: run ? runEntities(run) : [],
        main: split.main,
        folded: split.folded,
      }
    })
  })

  /** 最近一次 run（状态横幅用）。 */
  const latestRun = computed<AgentRunSummary | undefined>(() => runs.value[0])

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

  /** 失败 run 的内联备注（非成功终态且可解析出文案时返回；否则 null）。
   * 挂在对应 user 消息气泡下，让「哪一轮为什么挂了」在对话流里可追溯——
   * 横幅只展示最近一次 run，历史失败原因不再随新提交成功而消失。 */
  function runFailedNote(run: AgentRunSummary | undefined): string | null {
    if (!run) return null
    if (run.status !== 'failed' && run.status !== 'timeout' && run.status !== 'unknown') return null
    return runErrorText(run, t)
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
    // 附件随重试一并还原（否则"再跑一次这个日志"会静默丢掉文件）
    files.value = runFiles(run)
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
    nextTick(() => focusAtEnd())
  }

  // ── 会话切换清空（§4.2 三处清空差异对照表，按 mode 逐条复刻、不取并集）──
  // switch：停轮询 + 丢弃合帧 + 提交/流式态复位；messages/runs 不清（loadChat 覆盖）。
  // new：同 switch 但立即清 messages/runs；不停轮询（原实现无 stopPolling）。
  // delete：一律不动（删除后切换保留草稿/附件/提交态是现状行为）。
  function resetForSessionSwitch(mode: SessionSwitchMode) {
    if (mode === 'delete') return
    if (mode === 'switch') stopPolling()
    discardPendingRpcEvents()
    submittedRunId.value = null
    cancelling.value = false
    liveMessages.value = []
    historySnapshot.value = []
    if (mode === 'new') {
      messages.value = []
      runs.value = []
    }
  }

  // 新消息到达时自动滚动（用户接近底部时）
  watch(
    () => messages.value.length,
    () => scrollToBottomIfNear(),
  )

  // timer 随 composable 生命周期清理（风险 2）：合帧队列与轮询不泄漏
  onUnmounted(() => {
    stopPolling()
    discardPendingRpcEvents()
  })

  return {
    // 状态
    messages,
    messagesLoading,
    submitting,
    runs,
    queueInfo,
    scrollRef,
    submittedRunId,
    cancelling,
    liveMessages,
    historySnapshot,
    // computed
    activeRun,
    activeRunId,
    canStop,
    displayedMessages,
    isLiveMessage,
    latestRun,
    queueOccupiedBy,
    queueHint,
    userRunMap,
    messageDecorations,
    // 加载
    loadChat,
    // 流式（编排层事件桥入口）
    handleRpcStream,
    discardPendingRpcEvents,
    // 轮询
    startPolling,
    stopPolling,
    // 提交 / 停止 / 重试
    handleSubmit,
    handleCancel,
    onRunFinished,
    runFailedNote,
    handleRetry,
    handleRetryEdit,
    // 会话切换
    resetForSessionSwitch,
  }
}
