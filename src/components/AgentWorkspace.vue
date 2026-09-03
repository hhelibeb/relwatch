<script setup lang="ts">
import { ref, reactive, computed, inject, onMounted, onUnmounted, nextTick, watch, type ComponentPublicInstance } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { events } from '../bindings'
import { ShowToastKey, type AgentEntityRefSeed, type AgentWorkspaceSeed } from '../injection-keys'
import {
  getAgentConfig,
  getAgentAvailableModels,
  openAgentSession,
  getAgentSessionCommand,
  saveAgentConfig,
  type AgentQueueItem,
  type AgentRunSummary,
} from '../api/agent'
import { listSources, type Source } from '../api/sources'
import { getReleases, type ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import { useAgentUsage } from './agent/useAgentUsage'
import { useAgentRpc } from './agent/useAgentRpc'
import { useAgentModels } from './agent/useAgentModels'
import { useAgentComposer } from './agent/useAgentComposer'
import { useAgentSessions } from './agent/useAgentSessions'
import { useAgentChat } from './agent/useAgentChat'
import AgentRpcIndicator from './agent/AgentRpcIndicator.vue'
import AgentSessionSidebar from './agent/AgentSessionSidebar.vue'
import AgentRunBanner from './agent/AgentRunBanner.vue'
import AgentMessageList from './agent/AgentMessageList.vue'
import AgentComposer from './agent/AgentComposer.vue'

const props = defineProps<{ seed?: AgentWorkspaceSeed | null; width?: number }>()
const emit = defineEmits<{ close: [] }>()
const showToast = inject(ShowToastKey, () => {})
// 面板宽度：默认 440（CSS 兜底同值），App.vue 展开时传入持久化宽度
const panelWidth = computed(() => props.width ?? 440)

// ── 数据源：全局 skill 列表 + 实体目录（[[]] 菜单/名称映射）────
const skills = ref<string[]>([])
const sources = ref<Source[]>([])
const releases = ref<ReleaseInfo[]>([])

// ── 全局队列（侧栏状态点 / 横幅「被谁占用」）：跨域共享，编排层持有、chat 写入 ──
const queueActive = ref<AgentQueueItem[]>([])

// ── 引用与输入区（F 域 composable）：草稿/引用/菜单/chip 反馈由其持有 ──
const {
  instruction,
  entities,
  skillPath,
  files,
  textareaRef,
  showSkillMenu,
  showEntityMenu,
  skillMenuIndex,
  entityMenuIndex,
  flashKey,
  attachAnnouncement,
  chipTooltip,
  filteredSkills,
  filteredSources,
  filteredReleases,
  filteredSourcesCount,
  filteredReleasesCount,
  entityMenuHasMatch,
  handleInput: parseComposerInput,
  handleAttachFiles,
  removeFile,
  fileDisplayName,
  addEntity,
  afterAttach,
  removeEntity,
  entityLabel,
  entityKindLabel,
  handleChipEnter,
  handleChipMove,
  hideChipTooltip,
  sourceDisplayName,
  releaseDisplayName,
  pickSkill,
  clearSkill,
  pickEntity,
  resetForSessionSwitch: resetComposerForSessionSwitch,
  focus: focusComposer,
  focusAtEnd,
  closeMenus,
} = useAgentComposer({ showToast, skills, sources, releases })

// textarea 渲染在 AgentComposer 内：经函数 ref 回填 composable 的元素 ref
// （focus()/replaceTrigger 等经 textareaRef 读写光标）
function setTextareaEl(el: Element | ComponentPublicInstance | null) {
  textareaRef.value = el as HTMLTextAreaElement | null
}


// ── 会话管理（A 域 composable）：索引/发现/重命名/⋯菜单/删除/清理/搜索/侧栏折叠 ──
const {
  sessions,
  activeKey,
  sessionTitle,
  sessionQuery,
  sidebarOpen,
  toggleSidebar,
  discoverSessions,
  switchTo,
  registerNew,
  currentMeta,
  renamingKey,
  renameInput,
  startRename,
  commitRename,
  cancelRename,
  setRenameEl,
  openMenuKey,
  setSessionMoreEl,
  sessionMenuStyle,
  toggleSessionMenu,
  onSessionListScroll,
  handleDeleteFromMenu,
  handleExportSession,
  updateModel,
  persistSessionMeta,
  handleClearSessions,
  visibleSessions,
  sessionTitleOf,
  resetForSessionSwitch: resetSessionsForSessionSwitch,
} = useAgentSessions({
  showToast,
  queueActive,
  // 删除活跃会话后的跨域清空（原 handleDeleteSession 的 if 分支；§4.2 delete 列：
  // 不清 files/oneShotModel/modelOnce——删除会话后草稿/附件保留是现状行为；
  // chat 域 delete mode 不动，仅 loadChat 换新会话内容）。
  // 闭包引用下方 useAgentChat 解构的 loadChat（异步回调执行时已初始化）。
  onActiveDeleted: async () => {
    resetComposerForSessionSwitch('delete')
    resetModelsForSessionSwitch(
      'delete',
      sessions.value.find((s) => s.key === activeKey.value)?.model ?? null,
    )
    await loadChat()
  },
})

// ── 会话上下文水位（H 域 composable）──
const { usage, loadUsage, usageText, usageWarn } = useAgentUsage(activeKey)

// ── 模型选择（D 域 composable）：会话级落库经回调转调会话域，不互相 import ──
const {
  availableModels,
  currentModel,
  selectedModel,
  oneShotModel,
  modelOnce,
  effectiveModel,
  showModelMenu,
  modelMenuIndex,
  modelLabel,
  isModelSelected,
  activeModelLabel,
  modelDefaultSub,
  toggleModelMenu,
  pickModel,
  toggleModelOnce,
  resetForSessionSwitch: resetModelsForSessionSwitch,
} = useAgentModels({
  onPersistModel: (model) => updateModel(activeKey.value, model),
  // 模型菜单打开时收起引用菜单（同屏互斥，原 toggleModelMenu 行为）
  onMenuOpen: () => {
    showSkillMenu.value = false
    showEntityMenu.value = false
  },
})

// ── pi 进程健康（E 域 composable）：指示灯 + 状态菜单 + 重启 ──
const {
  rpcStatus,
  rpcRestarting,
  rpcMenuOpen,
  rpcDotEl,
  rpcMenuStyle,
  loadRpcStatus,
  toggleRpcMenu,
  rpcRestartPending,
  handleRestartRpc,
} = useAgentRpc({
  showToast,
  // rpc 菜单打开时收起输入区各菜单（同屏互斥，原 toggleRpcMenu 行为）
  onMenuOpen: () => {
    showModelMenu.value = false
    showSkillMenu.value = false
    showEntityMenu.value = false
  },
})
// 灯按钮渲染在 AgentRpcIndicator 内：经函数 ref 回填 composable 的锚点 ref
function setRpcDotEl(el: Element | ComponentPublicInstance | null) {
  rpcDotEl.value = el as HTMLElement | null
}
// ── 聊天核心（B + C 域 composable）：历史/流式合帧/轮询/滚动 + 提交/停止/重试 ──
// composer / models 的草稿与模型 ref、usage 句柄经入参注入（§4.2 ref 传递）；
// 会话域回调（sessionTitle / persistSessionMeta）引用上方 sessions 解构变量。
const {
  messages,
  messagesLoading,
  submitting,
  runs,
  scrollRef,
  cancelling,
  liveMessages,
  displayedMessages,
  isLiveMessage,
  latestRun,
  canStop,
  queueOccupiedBy,
  queueHint,
  messageDecorations,
  loadChat,
  handleRpcStream,
  handleSubmit,
  handleCancel,
  onRunFinished,
  runFailedNote,
  handleRetry,
  handleRetryEdit,
  resetForSessionSwitch: resetChatForSessionSwitch,
} = useAgentChat({
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
})

// 滚动容器元素回填（scrollRef 为 useAgentChat 持有的同一 ref）
function setChatScrollEl(el: Element | ComponentPublicInstance | null) {
  scrollRef.value = el as HTMLElement | null
}

/** 输入触发（textarea @input）：先收起模型菜单（原 handleInput 首句语义），
 *  再解析 @/[[ 触发词。原代码两组状态物理同域，拆分后经此包装保持行为一致。 */
function handleInput() {
  showModelMenu.value = false
  parseComposerInput()
}

// ── 会话切换组合（编排层接线；各域清空清单见设计文档 §4.2，按 mode 逐条复刻）──
function switchSession(key: string) {
  if (key === activeKey.value) return
  // 不中止原会话的 run：后端并发上限 1、其余排队执行（pending 取消只插标记不碰进程），
  // 切回会话时由 loadChat 从 runs 推导恢复停止按钮——各会话独立启停，互不误杀
  switchTo(key)
  // chat：停轮询 + 丢合帧 + 提交/流式态复位（messages/runs 不清，loadChat 覆盖）
  resetChatForSessionSwitch('switch')
  // 恢复的会话被打开过即转为普通会话（已确认，不再是异常态）；同时写回索引
  resetModelsForSessionSwitch(
    'switch',
    sessions.value.find((s) => s.key === key)?.model ?? null,
  )
  void loadChat()
  // composer：引用/指令清空 + 附件清空（「这一轮的输入」不跟着用户漂移到另一会话）
  resetComposerForSessionSwitch('switch')
  // sessions：重命名/⋯菜单收起
  resetSessionsForSessionSwitch('switch')
}

function startNewSession() {
  // 当前已是未提交草稿且无内容 → 不重复新建
  const cur = currentMeta()
  if (cur?.draft && messages.value.length === 0 && runs.value.length === 0) return
  // 新建即登记：立即写入索引并持久化，未提交的会话也可见、可恢复（评审 1.2）
  registerNew()
  resetComposerForSessionSwitch('new')
  resetModelsForSessionSwitch('new', null)
  resetSessionsForSessionSwitch('new')
  // chat：立即清 messages/runs + 流式/提交态复位（new 不停轮询，原实现即如此）
  resetChatForSessionSwitch('new')
  void loadChat()
  nextTick(() => focusComposer())
}


// ── 运行历史面板（评审 P1：耗时 / 模型 / 状态 / 引用实体）──
const historyOpen = ref(false)

// ── 超时引导（评审 P1：行动建议 + 就地调时长）──
const timeoutSecs = ref(300)
const adjustingTimeout = ref(false)
const timeoutInput = ref('')

function startAdjustTimeout() {
  adjustingTimeout.value = true
  timeoutInput.value = String(timeoutSecs.value)
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


// 横幅快捷操作（在 Agent 中打开 / 复制命令）折叠状态：默认折叠节省空间，点击 << 展开、>> 收起
const actionsExpanded = ref(false)

let unlistenRunFinished: UnlistenFn | undefined
let unlistenRpcStream: UnlistenFn | undefined
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
  } finally {
    // 模型枚举会惰性拉起常驻 pi 进程（Agent 启用时），它与挂载时的 loadRpcStatus
    // 并发，后者可能先于进程就绪返回 false，灯便停在过时的灰色快照上，直到用户
    // 点灯才变绿——观感像「点击把进程点出来了」。枚举结束后补查一次，
    // 让灯与实际进程状态一致（查询本身惰性，不额外拉起进程）
    void loadRpcStatus()
  }
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

/** 点菜单及其触发控件之外任意区域 → 收起当前打开的菜单（下拉菜单通用行为）。 */
function onDocumentPointerDown(e: MouseEvent | PointerEvent) {
  // 通过捕获期触发，确保在菜单项 @click 之前执行；只判断是否点到了「菜单或触发控件」内部，
  // 是则交由原逻辑（切换/选择）处理，否则一律收起，实现点击空白区域收起。
  const t = e.target as EventTarget | null
  if (!(t instanceof Element)) return
  if (t.closest('.agent-ws-menu')) return
  if (t.closest('.agent-ws-model-btn')) return // 模型菜单的触发按钮，交给 toggleModelMenu
  if (t.closest('.agent-ws-session-menu')) return // 会话 ⋯ 菜单
  if (t.closest('.agent-ws-session-more')) return // 会话 ⋯ 触发按钮，交给 toggleSessionMenu
  if (t.closest('.agent-ws-rpc-wrap')) return // pi 状态菜单及其触发灯，交给 toggleRpcMenu
  // 点输入框：技能/实体菜单跟随输入（显隐由输入框自身事件管理），保持不动；
  // 但与输入无关的模型菜单 / pi 状态菜单 / 会话菜单仍应收起——
  // 此前无条件 return 把它们一并豁免，导致点输入框时这些菜单悬而不收
  const inTextarea = t.closest('.agent-ws-textarea') !== null
  if (showModelMenu.value) showModelMenu.value = false
  // composer.closeMenus(excludeTextarea)：点输入框不收 skill/entity 菜单的豁免语义
  closeMenus(inTextarea)
  // pi 状态菜单：点空白即收起
  if (rpcMenuOpen.value) rpcMenuOpen.value = false
  // 会话菜单：点空白即收起（重命名输入框有自己的 Enter/Esc，不走这里）
  if (openMenuKey.value && !t.closest('.agent-ws-rename-input')) openMenuKey.value = null
}


// ── 子组件接线（props/emits 面较宽，对象展开收敛模板宽度；§4.4 接线宽度成本）──
// reactive 自动解包内部 ref/computed，与逐个 :prop 传值等价
// AgentRunBanner：状态横幅 + 推迟生效提示 + 历史面板
const runBannerProps = reactive({
  latestRun,
  queueHint,
  queueOccupiedBy,
  queueOccupiedByTitle: computed(() => (queueOccupiedBy.value ? sessionTitleOf(queueOccupiedBy.value) : '')),
  sessionTitle,
  runs,
  rpcRestartPending,
  switchSession,
  openSession: handleOpenSession,
  copySessionCommand: handleCopySessionCommand,
  retry: handleRetry,
})
const runBannerHandlers = {
  'update:historyOpen': (v: boolean) => (historyOpen.value = v),
  'update:actionsExpanded': (v: boolean) => (actionsExpanded.value = v),
}

// AgentSessionSidebar：会话侧栏展示 + ⋯菜单（Teleport 在子组件内）
const sidebarProps = reactive({
  sidebarOpen,
  sessions,
  visibleSessions,
  activeKey,
  renamingKey,
  openMenuKey,
  sessionMenuStyle,
  setRenameEl,
  setSessionMoreEl,
})
const sidebarHandlers = {
  switch: switchSession,
  scrollList: onSessionListScroll,
  commitRename,
  cancelRename,
  toggleMenu: toggleSessionMenu,
  exportSession: handleExportSession,
  rename: startRename,
  deleteFromMenu: handleDeleteFromMenu,
  clearSessions: handleClearSessions,
}

// AgentMessageList：消息区展示 + 超时引导
const messageListProps = reactive({
  messagesLoading,
  liveCount: computed(() => liveMessages.value.length),
  displayedMessages,
  messageDecorations,
  isLiveMessage,
  entityKindLabel,
  entityLabel,
  handleChipEnter,
  handleChipMove,
  hideChipTooltip,
  runFailedNote,
  retry: handleRetry,
  retryEdit: handleRetryEdit,
  adjustingTimeout,
  timeoutInput,
})
const messageListHandlers = {
  'update:adjustingTimeout': (v: boolean) => (adjustingTimeout.value = v),
  'update:timeoutInput': (v: string) => (timeoutInput.value = v),
  startAdjustTimeout,
  saveTimeout,
  cancelAdjustTimeout: () => (adjustingTimeout.value = false),
}

// AgentComposer：输入区展示 + 三个菜单 + 水位条
const composerProps = reactive({
  setTextareaEl,
  entities,
  files,
  skillPath,
  flashKey,
  attachAnnouncement,
  skills,
  showSkillMenu,
  showEntityMenu,
  skillMenuIndex,
  entityMenuIndex,
  filteredSkills,
  filteredSources,
  filteredReleases,
  filteredSourcesCount,
  filteredReleasesCount,
  entityMenuHasMatch,
  showModelMenu,
  modelMenuIndex,
  availableModels,
  effectiveModel,
  modelOnce,
  activeModelLabel,
  modelDefaultSub,
  submitting,
  canStop,
  cancelling,
  usageText,
  usageWarn,
  usage,
  chipTooltip,
  modelLabel,
  isModelSelected,
  entityLabel,
  entityKindLabel,
  fileDisplayName,
  sourceDisplayName,
  releaseDisplayName,
  handleChipEnter,
  handleChipMove,
  hideChipTooltip,
})
const composerHandlers = {
  'skill-hover': (i: number) => (skillMenuIndex.value = i),
  'entity-hover': (i: number) => (entityMenuIndex.value = i),
  'model-hover': (i: number) => (modelMenuIndex.value = i),
  submit: () => handleSubmit(),
  cancel: () => handleCancel(),
  input: handleInput,
  keydown: handleKeydown,
  attachFiles: handleAttachFiles,
  removeEntity,
  removeFile,
  clearSkill,
  pickSkill,
  pickEntity,
  toggleModelMenu,
  pickModel,
  toggleModelOnce,
  newSession: startNewSession,
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
      afterAttach(entity, addEntity(entity))
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
      afterAttach(entity, addEntity(entity))
    }
  } catch {
    // 非本应用拖入内容，忽略
  }
}

onMounted(async () => {
  applySeed()
  await Promise.all([loadCatalog(), loadChat(), loadRpcStatus()])
  // 磁盘发现放在首次加载之后：会话文件是索引的兜底来源，索引缺失时补入，
  // 补入的会话不打断当前激活会话（仅侧栏可见）
  const recovered = await discoverSessions()
  if (recovered > 0) showToast(t('agent.sessions_recovered', String(recovered)))
  unlistenRunFinished = await events.agentRunFinished.listen(() => {
    void onRunFinished()
  })
  unlistenRpcStream = await events.agentRpcStream.listen((e) => {
    handleRpcStream(e.payload)
  })
  await nextTick()
  focusComposer()
  // 捕获期监听：点击菜单/触发控件之外的区域即收起打开的下拉菜单
  document.addEventListener('pointerdown', onDocumentPointerDown, true)
})

// 面板打开期间 seed 更新（重复点「发送到 Agent」）：追加新实体
watch(
  () => props.seed,
  () => applySeed(),
)

onUnmounted(() => {
  unlistenRunFinished?.()
  unlistenRpcStream?.()
  document.removeEventListener('pointerdown', onDocumentPointerDown, true)
})

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
        <!-- pi 常驻进程健康指示：点灯弹状态菜单（状态详情 + 重启入口）。
             状态/菜单逻辑在 useAgentRpc（编排层持有），展示在本子组件。 -->
        <AgentRpcIndicator
          :rpc-status="rpcStatus"
          :rpc-restarting="rpcRestarting"
          :rpc-menu-open="rpcMenuOpen"
          :rpc-menu-style="rpcMenuStyle"
          :set-dot-el="setRpcDotEl"
          @toggle-menu="toggleRpcMenu"
          @restart="handleRestartRpc"
        />
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
        <!-- 最近 run 状态横幅 + 配置推迟生效提示 + 运行历史面板：
             状态来自 useAgentChat / useAgentRpc，展示在 AgentRunBanner -->
        <AgentRunBanner
          v-model:history-open="historyOpen"
          v-model:actions-expanded="actionsExpanded"
          v-bind="runBannerProps"
          v-on="runBannerHandlers"
        />

        <!-- 消息区：三种气泡形态 + messageDecorations 展示在 AgentMessageList；
             滚动容器 ref 经 scrollRef prop 回填 useAgentChat -->
        <AgentMessageList
          :set-scroll-el="setChatScrollEl"
          v-bind="messageListProps"
          v-on="messageListHandlers"
        />

        <!-- 输入区：chips 行 + textarea + 模型/附件/发送 + 三个菜单 + 水位条
             展示在 AgentComposer；状态在编排层 composable，动作/索引回写经 emit -->
        <AgentComposer
          v-model:instruction="instruction"
          v-bind="composerProps"
          @update:instruction="instruction = $event"
          v-on="composerHandlers"
        />
      </section>

      <!-- 右侧：会话侧栏（可折叠，折叠时聊天区占满全宽）：
           列表/搜索/重命名/⋯菜单（Teleport 在子组件内）展示在 AgentSessionSidebar -->
      <AgentSessionSidebar
        v-model:session-query="sessionQuery"
        v-model:rename-input="renameInput"
        v-bind="sidebarProps"
        v-on="sidebarHandlers"
      />
    </div>
  </div>
</template>

<!-- 菜单基类族（.agent-ws-menu*）非 scoped 共享：Teleport 到 body 的浮层菜单
     只带渲染组件的 data-v 属性，编排层 scoped 规则不再命中（见 agent-shared.css） -->
<style src="./agent/agent-shared.css"></style>
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

/* 聊天区（position: relative 供运行历史浮层 .agent-ws-history 定位） */
.agent-ws-chat {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
}

</style>
