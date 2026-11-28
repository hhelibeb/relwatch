import { computed, nextTick, ref, watch, type ComponentPublicInstance } from 'vue'
import { defineStore } from 'pinia'
import {
  AGENT_TIMEOUT_MAX,
  AGENT_TIMEOUT_MIN,
  getAgentAvailableModels,
  getAgentConfig,
  getAgentQueue,
  getAgentQueueStatus,
  getAgentSessionCommand,
  listAgentMessages,
  listAgentRuns,
  openAgentSession,
  saveAgentConfig,
  type AgentChatMessage,
  type AgentQueueItem,
  type AgentQueueStatus,
  type AgentRunSummary,
} from '../api/agent'
import { copyTextToClipboard } from '../api/client'
import { t } from '../i18n'
import type { AgentEntityRefSeed, AgentWorkspaceSeed } from '../types/agent'
import { extractErrorMessage } from '../services/feedback'
import { useCatalogStore } from './catalog'
import { useUiStore } from './ui'
import { useAgentComposer } from '../components/agent/useAgentComposer'
import { useAgentSessions, type SessionSwitchMode } from '../components/agent/useAgentSessions'
import { useAgentUsage } from '../components/agent/useAgentUsage'
import { useAgentModels } from '../components/agent/useAgentModels'
import { useAgentRpc } from '../components/agent/useAgentRpc'
import { createSessionCache, type SessionChatState } from '../components/agent/agentSessionCache'
import { createRpcStreamBatcher, type RpcStreamPayload } from '../components/agent/agentRpcStream'
import {
  buildUserRunMap,
  computeMessageDecorations,
  computeRunFailedNote,
} from '../components/agent/agentMessageDecorate'
import { createAgentRunActions } from '../components/agent/agentRunActions'

export const useAgentStore = defineStore('agent', () => {
  const ui = useUiStore()
  const catalog = useCatalogStore()

  // 面板级壳状态（由 App.vue 的 useAgentPanelLayout 同步，便于子组件消费）
  const panelOpen = ref(false)
  const panelSeed = ref<AgentWorkspaceSeed | null>(null)
  let openWorkspaceHandler: ((seed?: AgentWorkspaceSeed) => void) | null = null
  let toggleWorkspaceHandler: (() => void) | null = null

  function bindPanelHandlers(handlers: {
    open: (seed?: AgentWorkspaceSeed) => void
    toggle: () => void
  }): void {
    openWorkspaceHandler = handlers.open
    toggleWorkspaceHandler = handlers.toggle
  }

  function openWorkspace(seed?: AgentWorkspaceSeed): void {
    panelSeed.value = seed ?? null
    openWorkspaceHandler?.(seed)
  }

  function toggleWorkspace(): void {
    toggleWorkspaceHandler?.()
  }

  // ── 数据源（直接消费 catalogStore，零双套拉取路径）──
  const skills = ref<string[]>([])
  const sources = computed(() => catalog.sources)
  const releases = computed(() => catalog.releases)
  const catalogReady = computed(() => ({
    source: catalog.sourcesLoaded,
    release: catalog.releasesLoaded,
  }))

  const timeoutSecs = ref(300)
  const adjustingTimeout = ref(false)
  const timeoutInput = ref('')
  const historyOpen = ref(false)
  const actionsExpanded = ref(false)
  const queueActive = ref<AgentQueueItem[]>([])

  watch(
    () => catalog.agentConfig,
    (cfg) => {
      if (!cfg) return
      skills.value = cfg.skills
      timeoutSecs.value = cfg.timeout_seconds
    },
    { immediate: true },
  )

  // ── 输入区（Composer）──
  const composer = useAgentComposer({
    showToast: (msg) => ui.showToast(msg),
    skills,
    sources,
    releases,
  })

  const sessionsApi = useAgentSessions({
    showToast: (msg) => ui.showToast(msg),
    queueActive,
    onActiveDeleted: async () => {
      stopPolling()
      composer.resetForSessionSwitch('delete')
      modelsApi.resetForSessionSwitch(
        'delete',
        sessionsApi.sessions.value.find((s) => s.key === sessionsApi.activeKey.value)?.model ?? null,
      )
      await loadChat()
    },
  })

  // ── 上下文水位 ──
  const usageApi = useAgentUsage(sessionsApi.activeKey)

  // ── 模型选择 ──
  const modelsApi = useAgentModels({
    onPersistModel: (model) => sessionsApi.updateModel(sessionsApi.activeKey.value, model),
    onMenuOpen: () => {
      composer.showSkillMenu.value = false
      composer.showEntityMenu.value = false
    },
  })

  // ── RPC 进程状态 ──
  const rpcApi = useAgentRpc({
    showToast: (msg) => ui.showToast(msg),
    onMenuOpen: () => {
      modelsApi.showModelMenu.value = false
      composer.showSkillMenu.value = false
      composer.showEntityMenu.value = false
    },
  })

  // ── 多会话分片缓存 + LRU ──
  const sessionCache = createSessionCache(sessionsApi.activeKey)
  const cur = sessionCache.current

  const messages = computed({
    get: () => cur.value.messages,
    set: (v: AgentChatMessage[]) => { cur.value.messages = v },
  })
  const messagesLoading = computed({
    get: () => cur.value.messagesLoading,
    set: (v: boolean) => { cur.value.messagesLoading = v },
  })
  const submitting = computed({
    get: () => cur.value.submitting,
    set: (v: boolean) => { cur.value.submitting = v },
  })
  const runs = computed({
    get: () => cur.value.runs,
    set: (v: AgentRunSummary[]) => { cur.value.runs = v },
  })
  const queueInfo = computed({
    get: () => cur.value.queueInfo,
    set: (v: AgentQueueStatus | null) => { cur.value.queueInfo = v },
  })
  const liveMessages = computed({
    get: () => cur.value.liveMessages,
    set: (v: AgentChatMessage[]) => { cur.value.liveMessages = v },
  })
  const historySnapshot = computed({
    get: () => cur.value.historySnapshot,
    set: (v: AgentChatMessage[]) => { cur.value.historySnapshot = v },
  })
  const submittedRunId = computed({
    get: () => cur.value.submittedRunId,
    set: (v: number | null) => { cur.value.submittedRunId = v },
  })
  const cancelling = computed({
    get: () => cur.value.cancelling,
    set: (v: boolean) => { cur.value.cancelling = v },
  })

  const scrollRef = ref<HTMLElement | null>(null)

  function scrollToBottom(): void {
    nextTick(() => {
      const el = scrollRef.value
      if (el) el.scrollTop = el.scrollHeight
    })
  }

  function scrollToBottomIfNear(): void {
    const el = scrollRef.value
    if (!el) return
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 160
    if (nearBottom) scrollToBottom()
  }

  // ── 聊天数据拉取 ──
  const chatLoadEpoch = new Map<string, number>()

  function currentChatEpoch(key: string): number {
    return chatLoadEpoch.get(key) ?? 0
  }

  async function loadQueueInfo(
    key: string = sessionsApi.activeKey.value,
    epoch: number = currentChatEpoch(key),
  ): Promise<void> {
    try {
      const next = await getAgentQueueStatus(key)
      if (currentChatEpoch(key) !== epoch) return
      sessionCache.stateOf(key).queueInfo = next
    } catch {
      if (currentChatEpoch(key) !== epoch) return
      sessionCache.stateOf(key).queueInfo = null
    }
  }

  async function loadQueue(): Promise<void> {
    try {
      queueActive.value = await getAgentQueue()
    } catch {
      queueActive.value = []
    }
  }

  async function loadRuns(
    key: string = sessionsApi.activeKey.value,
    epoch: number = currentChatEpoch(key),
  ): Promise<void> {
    try {
      const next = await listAgentRuns(key, 50)
      if (currentChatEpoch(key) !== epoch) return
      sessionCache.stateOf(key).runs = next
    } catch {
      if (currentChatEpoch(key) !== epoch) return
      sessionCache.stateOf(key).runs = []
    }
  }

  async function loadMessages(
    key: string = sessionsApi.activeKey.value,
    epoch: number = currentChatEpoch(key),
  ): Promise<void> {
    try {
      const next = await listAgentMessages(key)
      if (currentChatEpoch(key) !== epoch) return
      sessionCache.stateOf(key).messages = next
    } catch {
      // 新会话可能尚无会话文件
    }
  }

  async function loadChatFor(key: string): Promise<void> {
    const st = sessionCache.stateOf(key)
    const epoch = currentChatEpoch(key) + 1
    chatLoadEpoch.set(key, epoch)
    const isActive = key === sessionsApi.activeKey.value
    if (isActive) {
      st.messagesLoading = true
      usageApi.usage.value = null
    }
    try {
      const tasks: Promise<void>[] = [
        loadRuns(key, epoch),
        loadMessages(key, epoch),
        loadQueueInfo(key, epoch),
        loadQueue(),
      ]
      if (isActive) tasks.push(usageApi.loadUsage(key))
      await Promise.all(tasks)
      if (currentChatEpoch(key) !== epoch) return
      st.submittedRunId = null
      const hasActive = st.runs.some((r) => r.status === 'running' || r.status === 'pending')
      if (!hasActive) st.activeStreamRunId = null
      if (hasActive && st.liveMessages.length === 0) {
        st.historySnapshot = [...st.messages]
      }
    } finally {
      if (currentChatEpoch(key) === epoch) {
        st.messagesLoading = false
        if (key === sessionsApi.activeKey.value) {
          scrollToBottom()
        }
      }
    }
  }

  function loadChat(): Promise<void> {
    return loadChatFor(sessionsApi.activeKey.value)
  }

  let pollTimer: ReturnType<typeof setInterval> | undefined
  let pollDeadline: number | undefined

  function stopPolling(): void {
    if (pollTimer) {
      clearInterval(pollTimer)
      pollTimer = undefined
    }
    pollDeadline = undefined
  }

  function startPolling(): void {
    stopPolling()
    pollDeadline = Date.now() + 120_000
    pollTimer = setInterval(async () => {
      if (pollDeadline && Date.now() > pollDeadline) {
        stopPolling()
        return
      }
      const tasks: Promise<void>[] = [loadQueue()]
      if (liveMessages.value.length === 0) tasks.push(loadMessages())
      await Promise.all(tasks)
      scrollToBottomIfNear()
    }, 1500)
    void rpcApi.loadRpcStatus()
  }

  const rpcBatcher = createRpcStreamBatcher({
    stateOf: sessionCache.stateOf,
    onSettled: (sessionKey) => {
      if (sessionKey === sessionsApi.activeKey.value) stopPolling()
      void loadChatFor(sessionKey)
    },
    onBatchFlushed: () => scrollToBottomIfNear(),
  })

  const workspaceMounted = ref(false)

  function handleRpcStream(payload: RpcStreamPayload): void {
    if (!workspaceMounted.value && !panelOpen.value && !sessionCache.hasState(payload.session_key)) {
      return
    }
    rpcBatcher.enqueue(payload)
  }

  function discardPendingRpcEvents(sessionKey?: string): void {
    rpcBatcher.discard(sessionKey)
  }

  // ── 派生视图 ──
  const activeRun = computed<AgentRunSummary | undefined>(() =>
    cur.value.runs.find((r) => r.status === 'running') ?? cur.value.runs.find((r) => r.status === 'pending'),
  )
  const activeRunId = computed<number | null>(() => activeRun.value?.id ?? cur.value.submittedRunId)
  const canStop = computed(() => activeRunId.value !== null)

  const displayedMessages = computed(() =>
    liveMessages.value.length > 0
      ? [...historySnapshot.value, ...liveMessages.value]
      : messages.value,
  )

  function isLiveMessage(msg: AgentChatMessage): boolean {
    return liveMessages.value.indexOf(msg) >= 0
  }

  const userRunMap = computed(() => buildUserRunMap(messages.value, runs.value))
  const messageDecorations = computed(() =>
    computeMessageDecorations(displayedMessages.value, runs.value, userRunMap.value),
  )

  const latestRun = computed<AgentRunSummary | undefined>(() => runs.value[0])

  const queueOccupiedBy = computed<string | null>(() => {
    const q = queueInfo.value
    if (!q?.other_running || latestRun.value?.status !== 'pending') return null
    if (q.max_concurrency > 1 && q.running_count < q.max_concurrency) return null
    const key = q.running_sessions[0]
    return key && key !== sessionsApi.activeKey.value ? key : null
  })

  const queueOccupiedByTitle = computed(() =>
    queueOccupiedBy.value ? sessionsApi.sessionTitleOf(queueOccupiedBy.value) : '',
  )

  const queueHint = computed<string | null>(() => {
    const q = queueInfo.value
    if (!q || latestRun.value?.status !== 'pending') return null
    if (q.max_concurrency > 1) {
      if (q.running_count < q.max_concurrency) return null
      return q.position && q.position > 1
        ? t('agent.queue_limit_reached_pos', String(q.position))
        : t('agent.queue_limit_reached')
    }
    if (q.other_running) {
      return q.position && q.position > 1
        ? t('agent.queue_other_running_pos', String(q.position))
        : t('agent.queue_other_running')
    }
    return q.position && q.position > 1 ? t('agent.queue_position', String(q.position)) : null
  })

  const runActions = createAgentRunActions({
    activeKey: sessionsApi.activeKey,
    stateOf: sessionCache.stateOf,
    showToast: (msg) => ui.showToast(msg),
    instruction: composer.instruction,
    entities: composer.entities,
    skillPath: composer.skillPath,
    files: composer.files,
    focusAtEnd: composer.focusAtEnd,
    effectiveModel: modelsApi.effectiveModel,
    selectedModel: modelsApi.selectedModel,
    oneShotModel: modelsApi.oneShotModel,
    modelOnce: modelsApi.modelOnce,
    sessionTitle: sessionsApi.sessionTitle,
    persistSessionMeta: sessionsApi.persistSessionMeta,
    showSkillMenu: composer.showSkillMenu,
    showEntityMenu: composer.showEntityMenu,
    sources,
    releases,
    catalogReady,
    activeRunId,
    canStop,
    submitting,
    loadChat,
    startPolling,
    discardPendingRpcEvents,
  })

  /**
   * 迟到的旧 run 终态：它已经被更新的 run 取代，只能刷新运行记录，
   * 不能清流式缓冲（那会把新 run 正在输出的回答抹掉）。
   */
  function isStaleRunFinished(st: SessionChatState, runId: number): boolean {
    return st.activeStreamRunId !== null && runId < st.activeStreamRunId
  }

  async function onRunFinished(sessionKey: string, runId?: number): Promise<void> {
    const st = sessionCache.getState(sessionKey)
    if (st) {
      if (runId !== undefined && isStaleRunFinished(st, runId)) {
        await loadRuns(sessionKey, currentChatEpoch(sessionKey))
      } else {
        if (sessionKey === sessionsApi.activeKey.value) stopPolling()
        st.cancelling = false
        discardPendingRpcEvents(sessionKey)
        st.activeStreamRunId = null
        st.liveMessages = []
        st.historySnapshot = []
        await loadChatFor(sessionKey)
      }
    }
    await loadQueue()
    if (sessionCache.hasState(sessionsApi.activeKey.value)) await loadQueueInfo(sessionsApi.activeKey.value)
    await rpcApi.loadRpcStatus()
  }

  function resetChatForSessionSwitch(mode: SessionSwitchMode): void {
    if (mode === 'delete') return
    if (mode === 'switch') {
      stopPolling()
      return
    }
    sessionCache.resetState(sessionsApi.activeKey.value)
  }

  function switchSession(key: string): void {
    if (key === sessionsApi.activeKey.value) return
    sessionsApi.switchTo(key)
    resetChatForSessionSwitch('switch')
    modelsApi.resetForSessionSwitch(
      'switch',
      sessionsApi.sessions.value.find((s) => s.key === key)?.model ?? null,
    )
    void loadChat()
    composer.resetForSessionSwitch('switch')
    sessionsApi.resetForSessionSwitch('switch')
  }

  function startNewSession(): void {
    const curMeta = sessionsApi.currentMeta()
    if (curMeta?.draft && messages.value.length === 0 && runs.value.length === 0) return
    sessionsApi.registerNew()
    composer.resetForSessionSwitch('new')
    modelsApi.resetForSessionSwitch('new', null)
    sessionsApi.resetForSessionSwitch('new')
    resetChatForSessionSwitch('new')
    void loadChat()
    nextTick(() => composer.focus())
  }

  // ── 模型与目录加载 ──
  async function loadModels(): Promise<void> {
    try {
      const info = await getAgentAvailableModels()
      modelsApi.availableModels.value = info.models
      modelsApi.currentModel.value = info.current
    } catch {
      modelsApi.availableModels.value = []
      modelsApi.currentModel.value = null
    } finally {
      void rpcApi.loadRpcStatus()
    }
  }

  async function loadCatalog(): Promise<void> {
    if (!catalog.agentConfig) {
      await catalog.loadAgentConfig()
    }
    if (!catalog.sourcesLoaded || !catalog.releasesLoaded) {
      await Promise.allSettled([catalog.loadSources(), catalog.loadReleases()])
    }
    await loadModels()
  }

  function ensureEntityKnown(entity: AgentEntityRefSeed): void {
    const known =
      entity.kind === 'source'
        ? sources.value.some((s) => s.id === entity.id)
        : releases.value.some((r) => r.id === entity.id)
    if (!known) catalog.scheduleRefreshCatalog()
  }

  function applySeed(seed: AgentWorkspaceSeed | null | undefined): void {
    if (!seed?.entities?.length) return
    for (const e of seed.entities) {
      ensureEntityKnown(e)
      composer.addEntity(e)
    }
  }

  // ── 终端会话操作 ──
  async function handleOpenSession(run: AgentRunSummary): Promise<void> {
    if (!run.session_path) return
    try {
      await openAgentSession(run.id)
    } catch (e) {
      ui.showToast(extractErrorMessage(e))
    }
  }

  async function handleCopySessionCommand(run: AgentRunSummary): Promise<void> {
    if (!run.session_path) return
    try {
      const cmd = await getAgentSessionCommand(run.id)
      await copyTextToClipboard(cmd)
      ui.showToast(t('agent.command_copied'))
    } catch (e) {
      ui.showToast(extractErrorMessage(e))
    }
  }

  // ── 超时就地调整 ──
  function startAdjustTimeout(): void {
    adjustingTimeout.value = true
    timeoutInput.value = String(timeoutSecs.value)
  }

  function cancelAdjustTimeout(): void {
    adjustingTimeout.value = false
  }

  async function saveTimeout(): Promise<void> {
    const v = Number(timeoutInput.value)
    if (!Number.isInteger(v) || v < AGENT_TIMEOUT_MIN || v > AGENT_TIMEOUT_MAX) {
      ui.showToast(t('agent.timeout_range'))
      return
    }
    try {
      const cfg = await getAgentConfig()
      cfg.timeout_seconds = v
      await saveAgentConfig(cfg)
      void catalog.loadAgentConfig()
      timeoutSecs.value = v
      adjustingTimeout.value = false
      ui.showToast(t('agent.timeout_saved', String(v)))
    } catch (e) {
      ui.showToast(extractErrorMessage(e))
    }
  }

  // ── 输入与菜单交互 ──
  function handleComposerInput(): void {
    modelsApi.showModelMenu.value = false
    composer.handleInput()
  }

  function handleMenuKeydown(e: KeyboardEvent): void {
    if (modelsApi.showModelMenu.value) {
      const total = 1 + modelsApi.availableModels.value.length
      if (total > 1) {
        if (e.key === 'ArrowDown') {
          e.preventDefault()
          modelsApi.modelMenuIndex.value = (modelsApi.modelMenuIndex.value + 1) % total
        } else if (e.key === 'ArrowUp') {
          e.preventDefault()
          modelsApi.modelMenuIndex.value = (modelsApi.modelMenuIndex.value - 1 + total) % total
        } else if (e.key === 'Enter' || e.key === 'Tab') {
          e.preventDefault()
          if (modelsApi.modelMenuIndex.value === 0) {
            modelsApi.pickModel(null)
          } else {
            const m = modelsApi.availableModels.value[modelsApi.modelMenuIndex.value - 1]
            if (m) modelsApi.pickModel(m)
          }
        } else if (e.key === 'Escape') {
          e.preventDefault()
          modelsApi.showModelMenu.value = false
        }
      }
      return
    }
    if (composer.showSkillMenu.value) {
      const total = composer.filteredSkills.value.length
      if (total === 0) return
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        composer.skillMenuIndex.value = (composer.skillMenuIndex.value + 1) % total
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        composer.skillMenuIndex.value = (composer.skillMenuIndex.value - 1 + total) % total
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        const s = composer.filteredSkills.value[composer.skillMenuIndex.value]
        if (s) composer.pickSkill(s)
      } else if (e.key === 'Escape') {
        e.preventDefault()
        composer.showSkillMenu.value = false
      }
      return
    }
    if (composer.showEntityMenu.value) {
      const total = composer.filteredSourcesCount.value + composer.filteredReleasesCount.value
      if (total === 0) return
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        composer.entityMenuIndex.value = (composer.entityMenuIndex.value + 1) % total
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        composer.entityMenuIndex.value = (composer.entityMenuIndex.value - 1 + total) % total
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        const i = composer.entityMenuIndex.value
        const s = composer.filteredSources.value.length
        if (i < s) {
          const src = composer.filteredSources.value[i]
          composer.pickEntity('source', src.id)
        } else {
          const rel = composer.filteredReleases.value[i - s]
          if (rel) composer.pickEntity('release', rel.id)
        }
      } else if (e.key === 'Escape') {
        e.preventDefault()
        composer.showEntityMenu.value = false
      }
    }
  }

  function handleComposerKeydown(e: KeyboardEvent): void {
    if (e.isComposing) return
    if (composer.showSkillMenu.value || composer.showEntityMenu.value || modelsApi.showModelMenu.value) {
      handleMenuKeydown(e)
      return
    }
    if (e.key === 'Enter' && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
      e.preventDefault()
      if (canStop.value || submitting.value) {
        ui.showToast(t('agent.enter_while_running'))
        return
      }
      void runActions.handleSubmit()
    }
  }

  function handleDocumentPointerDown(e: MouseEvent | PointerEvent): void {
    const target = e.target as EventTarget | null
    if (!(target instanceof Element)) return
    if (target.closest('.agent-ws-menu')) return
    if (target.closest('.agent-ws-model-btn')) return
    if (target.closest('.agent-ws-session-menu')) return
    if (target.closest('.agent-ws-session-more')) return
    if (target.closest('.agent-ws-rpc-wrap')) return
    const inTextarea = target.closest('.agent-ws-textarea') !== null
    if (modelsApi.showModelMenu.value) modelsApi.showModelMenu.value = false
    composer.closeMenus(inTextarea)
    if (rpcApi.rpcMenuOpen.value) rpcApi.rpcMenuOpen.value = false
    if (sessionsApi.openMenuKey.value && !target.closest('.agent-ws-rename-input')) {
      sessionsApi.openMenuKey.value = null
    }
  }

  // DOM 句柄注册（子组件内部挂 ref 时直接写回 store，无需经父组件透传函数 ref）
  function bindTextareaEl(el: Element | ComponentPublicInstance | null): void {
    composer.textareaRef.value = el as HTMLTextAreaElement | null
  }
  function bindRpcDotEl(el: Element | ComponentPublicInstance | null): void {
    rpcApi.rpcDotEl.value = el as HTMLElement | null
  }
  function bindChatScrollEl(el: Element | ComponentPublicInstance | null): void {
    scrollRef.value = el as HTMLElement | null
  }

  function mountWorkspaceRuntime(): void {
    workspaceMounted.value = true
  }

  function cleanupWorkspaceRuntime(): void {
    workspaceMounted.value = false
    stopPolling()
    rpcBatcher.flush()
    rpcApi.rpcMenuOpen.value = false
    sessionsApi.openMenuKey.value = null
    sessionsApi.renamingKey.value = null
    modelsApi.showModelMenu.value = false
    composer.showSkillMenu.value = false
    composer.showEntityMenu.value = false
    composer.chipTooltip.value = null
    historyOpen.value = false
    adjustingTimeout.value = false
    actionsExpanded.value = false
    sessionCache.pruneInactive()
  }

  watch(
    () => messages.value.length,
    () => scrollToBottomIfNear(),
  )

  watch(sessionsApi.activeKey, () => sessionCache.prune())

  return {
    // 壳状态与跨组件打开入口
    panelOpen,
    panelSeed,
    bindPanelHandlers,
    openWorkspace,
    toggleWorkspace,

    // 目录与基础配置
    skills,
    sources,
    releases,
    catalogReady,
    queueActive,
    timeoutSecs,
    adjustingTimeout,
    timeoutInput,
    historyOpen,
    actionsExpanded,
    startAdjustTimeout,
    cancelAdjustTimeout,
    saveTimeout,
    loadCatalog,
    loadModels,
    ensureEntityKnown,
    applySeed,

    // Composer 域
    instruction: composer.instruction,
    entities: composer.entities,
    skillPath: composer.skillPath,
    files: composer.files,
    textareaRef: composer.textareaRef,
    showSkillMenu: composer.showSkillMenu,
    showEntityMenu: composer.showEntityMenu,
    skillMenuIndex: composer.skillMenuIndex,
    entityMenuIndex: composer.entityMenuIndex,
    flashKey: composer.flashKey,
    attachAnnouncement: composer.attachAnnouncement,
    chipTooltip: composer.chipTooltip,
    filteredSkills: composer.filteredSkills,
    filteredSources: composer.filteredSources,
    filteredReleases: composer.filteredReleases,
    filteredSourcesCount: composer.filteredSourcesCount,
    filteredReleasesCount: composer.filteredReleasesCount,
    entityMenuHasMatch: composer.entityMenuHasMatch,
    handleAttachFiles: composer.handleAttachFiles,
    removeFile: composer.removeFile,
    fileDisplayName: composer.fileDisplayName,
    addEntity: composer.addEntity,
    afterAttach: composer.afterAttach,
    removeEntity: composer.removeEntity,
    entityLabel: composer.entityLabel,
    entityKindLabel: composer.entityKindLabel,
    handleChipEnter: composer.handleChipEnter,
    handleChipMove: composer.handleChipMove,
    hideChipTooltip: composer.hideChipTooltip,
    sourceDisplayName: composer.sourceDisplayName,
    releaseDisplayName: composer.releaseDisplayName,
    pickSkill: composer.pickSkill,
    clearSkill: composer.clearSkill,
    pickEntity: composer.pickEntity,
    focusComposer: composer.focus,
    focusAtEnd: composer.focusAtEnd,
    handleComposerInput,
    handleComposerKeydown,
    handleDocumentPointerDown,
    bindTextareaEl,

    // Sessions 域
    sessions: sessionsApi.sessions,
    activeKey: sessionsApi.activeKey,
    sessionTitle: sessionsApi.sessionTitle,
    sessionQuery: sessionsApi.sessionQuery,
    sidebarOpen: sessionsApi.sidebarOpen,
    toggleSidebar: sessionsApi.toggleSidebar,
    discoverSessions: sessionsApi.discoverSessions,
    renamingKey: sessionsApi.renamingKey,
    renameInput: sessionsApi.renameInput,
    startRename: sessionsApi.startRename,
    commitRename: sessionsApi.commitRename,
    cancelRename: sessionsApi.cancelRename,
    setRenameEl: sessionsApi.setRenameEl,
    openMenuKey: sessionsApi.openMenuKey,
    setSessionMoreEl: sessionsApi.setSessionMoreEl,
    sessionMenuStyle: sessionsApi.sessionMenuStyle,
    toggleSessionMenu: sessionsApi.toggleSessionMenu,
    onSessionListScroll: sessionsApi.onSessionListScroll,
    handleDeleteFromMenu: sessionsApi.handleDeleteFromMenu,
    handleExportSession: sessionsApi.handleExportSession,
    handleClearSessions: sessionsApi.handleClearSessions,
    visibleSessions: sessionsApi.visibleSessions,
    sessionTitleOf: sessionsApi.sessionTitleOf,
    switchSession,
    startNewSession,

    // Usage 域
    usage: usageApi.usage,
    loadUsage: usageApi.loadUsage,
    usageText: usageApi.usageText,
    usageEstimated: usageApi.usageEstimated,
    usageHint: usageApi.usageHint,

    // Models 域
    availableModels: modelsApi.availableModels,
    currentModel: modelsApi.currentModel,
    selectedModel: modelsApi.selectedModel,
    oneShotModel: modelsApi.oneShotModel,
    modelOnce: modelsApi.modelOnce,
    effectiveModel: modelsApi.effectiveModel,
    showModelMenu: modelsApi.showModelMenu,
    modelMenuIndex: modelsApi.modelMenuIndex,
    modelLabel: modelsApi.modelLabel,
    isModelSelected: modelsApi.isModelSelected,
    activeModelLabel: modelsApi.activeModelLabel,
    modelDefaultSub: modelsApi.modelDefaultSub,
    toggleModelMenu: modelsApi.toggleModelMenu,
    pickModel: modelsApi.pickModel,
    toggleModelOnce: modelsApi.toggleModelOnce,

    // RPC 域
    rpcStatus: rpcApi.rpcStatus,
    rpcRestarting: rpcApi.rpcRestarting,
    rpcMenuOpen: rpcApi.rpcMenuOpen,
    rpcMenuStyle: rpcApi.rpcMenuStyle,
    loadRpcStatus: rpcApi.loadRpcStatus,
    toggleRpcMenu: rpcApi.toggleRpcMenu,
    rpcRestartPending: rpcApi.rpcRestartPending,
    handleRestartRpc: rpcApi.handleRestartRpc,
    bindRpcDotEl,

    // Chat 域
    messages,
    messagesLoading,
    submitting,
    runs,
    queueInfo,
    liveMessages,
    historySnapshot,
    submittedRunId,
    cancelling,
    scrollRef,
    bindChatScrollEl,
    activeRun,
    activeRunId,
    canStop,
    displayedMessages,
    isLiveMessage,
    messageDecorations,
    latestRun,
    queueOccupiedBy,
    queueOccupiedByTitle,
    queueHint,
    runFailedNote: computeRunFailedNote,
    loadChat,
    handleRpcStream,
    discardPendingRpcEvents,
    onRunFinished,
    handleSubmit: runActions.handleSubmit,
    handleCancel: runActions.handleCancel,
    handleRetry: runActions.handleRetry,
    handleRetryEdit: runActions.handleRetryEdit,
    handleOpenSession,
    handleCopySessionCommand,
    mountWorkspaceRuntime,
    cleanupWorkspaceRuntime,
    startPolling,
    stopPolling,
    resetForSessionSwitch: resetChatForSessionSwitch,
  }
})
