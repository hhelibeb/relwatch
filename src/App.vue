<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, provide, watch, shallowRef, type Component, type Ref } from 'vue'
import { ShowToastKey, AiEnabledKey, ShowImportanceKey, AgentEnabledKey, AgentWorkspaceKey, AgentPanelOpenKey, AgentToggleKey, type AgentWorkspaceSeed } from './injection-keys'
import ContextMenu, { type ContextMenuItem } from './components/common/ContextMenu.vue'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { events, commands } from './bindings'
import { type Source, listSources, sourceRepoKey, syncSourceCapabilities } from './api/sources'
import { type ReleaseInfo, triggerPoll, getPollCountdown, getReleaseCatalog } from './api/releases'
import { copyTextToClipboard } from './api/client'
import { setErrorToastSink } from './api/report-error'
import { type AppSettings, getSettings, DEFAULT_SETTINGS } from './api/settings'
import { t, setLocale } from './i18n'
import { registerCloser, unregisterCloser, closeAllContextMenus } from './composables/contextMenuBus'
import { useTransientUiGuard } from './composables/useTransientUiGuard'
import { useEscapeToTray } from './composables/useEscapeToTray'
import { useExternalLinkGuard } from './composables/useExternalLinkGuard'
import { applyTheme } from './composables/useTheme'
import { applyFontScale } from './composables/useFontScale'
import { setUsageTrackingEnabled, flushUsageTrackingNow, track } from './composables/useUsageTracking'
import { isUnreadStatus, formatCountdown } from './utils'
import SourceTab from './components/SourceTab.vue'
import ReleaseTab from './components/ReleaseTab.vue'
import LogTab from './components/LogTab.vue'
import SettingsTab from './components/SettingsTab.vue'
import AgentWorkspace from './components/AgentWorkspace.vue'
import { getCurrentWindow, LogicalSize, currentMonitor } from '@tauri-apps/api/window'
import { getAgentConfig, type AgentConfig } from './api/agent'

const activeTab = ref<'sources' | 'releases' | 'logs' | 'settings'>('sources')
const mainScrolled = ref(false)

function onMainScroll(e: Event) {
  const el = e.currentTarget as HTMLElement
  mainScrolled.value = el.scrollTop > 0
}

const sources = ref<Source[]>([])
const releases = ref<ReleaseInfo[]>([])
/* 加载态与错误态：列表为空时用来把「正在拉取」「拉取失败」「真的没有数据」
   三种情况分开（此前三者都渲染同一个空态）。 */
const sourcesLoading = ref(false)
const sourcesError = ref('')
const releasesLoading = ref(false)
const releasesError = ref('')
const logRefreshKey = ref(0)
const settings = ref<AppSettings>({ ...DEFAULT_SETTINGS })
// Agent 全局配置（独立于 AppSettings：含 JSON 数组字段，不走设置注册表）
const agentConfig = ref<AgentConfig | null>(null)

// ── Agent 工作区右栏：三个宽度、一份账 ──
//   L = 主界面列宽（mainWidth，记住的）
//   R = Agent 面板宽（agentPanelWidth，记住的，持久化到 app_settings.agent_ws_width）
//   W = 窗口宽（由原生窗口决定）
// 不变式：面板收起时 W == L；面板展开时 W == L + 分隔线 + R。
//   · 面板收起：拖窗口边缘改的是 L，R 不动 → 下次打开仍按上次的 R 展开
//   · 面板展开：拖窗口边缘改的是 R（L 不动）；拖分隔条时 L、R 同时变，两者都记住
//   · 打开：W → L + 分隔线 + R；关闭：W → L（主界面宽度不因开关而变化）
const AGENT_PANEL_DEFAULT_WIDTH = 440
const AGENT_PANEL_MIN_WIDTH = 280
const MAIN_MIN_WIDTH = 710
// 与 src-tauri/src/lib.rs 的 min_inner_size 保持同步
const MAIN_MIN_HEIGHT = 450
const AGENT_DIVIDER_WIDTH = 5
const agentPanelOpen = ref(false)
const agentPanelSeed = ref<AgentWorkspaceSeed | null>(null)
const mainWidth = ref(window.innerWidth)
// 视口宽（布局 px）：窗口边缘拖拽时随原生 resize 更新
const viewportWidth = ref(window.innerWidth)
// 面板宽 = 视口 − 主界面 − 分隔线（面板收起时不参与渲染，也不参与持久化）
const agentPanelWidth = computed(() =>
  Math.max(AGENT_PANEL_MIN_WIDTH, viewportWidth.value - mainWidth.value - AGENT_DIVIDER_WIDTH),
)
// 开关互斥锁：innerSize/setSize 是异步的，防快速连点导致交错竞态
let panelBusy = false
let panelWidthSaveTimer: ReturnType<typeof setTimeout> | null = null

function onViewportResize() {
  viewportWidth.value = window.innerWidth
  // 开/关途中：setSize 自己触发的 resize 不能反写 L，L 由 open/close 算完再落定
  if (panelBusy) return
  if (!agentPanelOpen.value) {
    // 面板收起：窗口宽度就是主界面宽度，跟着改；下次展开的面板宽不受影响
    mainWidth.value = viewportWidth.value
    return
  }
  // 面板展开：窗口边缘的拖拽只改面板宽度，改完要记住
  schedulePanelWidthSave()
}

function clampMainWidth(w: number, totalW: number): number {
  // 下限保证主界面可读；上限给面板留出最小宽度
  const maxW = Math.max(MAIN_MIN_WIDTH, totalW - AGENT_DIVIDER_WIDTH - AGENT_PANEL_MIN_WIDTH)
  return Math.round(Math.min(Math.max(w, MAIN_MIN_WIDTH), maxW))
}

/** 面板展开期间窗口的最小宽度：主界面列宽 + 分隔线 + 面板最小宽度 */
function agentWindowMinWidth(mainW: number): number {
  return mainW + AGENT_DIVIDER_WIDTH + AGENT_PANEL_MIN_WIDTH
}

function cancelPanelWidthSave() {
  if (panelWidthSaveTimer) {
    clearTimeout(panelWidthSaveTimer)
    panelWidthSaveTimer = null
  }
}

function persistPanelWidth(w: number) {
  void commands.saveAgentWsWidth(w).catch(() => {})
}

function schedulePanelWidthSave() {
  cancelPanelWidthSave()
  const w = agentPanelWidth.value
  panelWidthSaveTimer = setTimeout(() => {
    panelWidthSaveTimer = null
    persistPanelWidth(w)
  }, 600)
}

// ── 分隔线拖拽：只改主界面列宽，面板吸收差额（窗口总宽不变）──
let dividerDragging = false
let dividerStartX = 0
let dividerStartMainWidth = 0

function onDividerMouseDown(e: MouseEvent) {
  e.preventDefault()
  dividerDragging = true
  dividerStartX = e.clientX
  // 窗口被挤窄时渲染宽度已小于 mainWidth：先对齐再起拖，否则开头一段拖动是死的
  dividerStartMainWidth = clampMainWidth(mainWidth.value, viewportWidth.value)
  mainWidth.value = dividerStartMainWidth
  document.addEventListener('mousemove', onDividerMouseMove)
  document.addEventListener('mouseup', onDividerMouseUp)
  document.body.classList.add('agent-resizing')
}

function onDividerMouseMove(e: MouseEvent) {
  if (!dividerDragging) return
  // 向右拖 → 主界面变宽、面板变窄；向左拖反之
  mainWidth.value = clampMainWidth(
    dividerStartMainWidth + (e.clientX - dividerStartX),
    viewportWidth.value,
  )
}

function onDividerMouseUp() {
  if (!dividerDragging) return
  dividerDragging = false
  document.removeEventListener('mousemove', onDividerMouseMove)
  document.removeEventListener('mouseup', onDividerMouseUp)
  document.body.classList.remove('agent-resizing')
  // L 变了：面板宽记住，窗口最小宽度跟着抬高（面板 280 的保底）
  schedulePanelWidthSave()
  syncWindowMinWidth()
}

async function loadSavedPanelWidth(): Promise<number> {
  try {
    const saved = await commands.getAgentWsWidth()
    return saved > 0 ? saved : AGENT_PANEL_DEFAULT_WIDTH
  } catch {
    return AGENT_PANEL_DEFAULT_WIDTH
  }
}

// 最近一次下发成功的窗口最小宽度：去重用，避免每像素一次 IPC
let lastMinWidth = -1

/**
 * 下发窗口最小宽度。面板展开期间下限抬到 L + 分隔线 + 面板 280（主界面列宽才不会被
 * 窗口边缘拖拽压扁），收起时回到 Rust 侧的 710。
 */
async function setWindowMinWidth(win: ReturnType<typeof getCurrentWindow>, width: number) {
  if (width === lastMinWidth) return
  lastMinWidth = width
  try {
    await win.setMinSize(new LogicalSize(width, MAIN_MIN_HEIGHT))
  } catch {
    // 权限缺失/平台不支持：退回「窗口太窄时先压主界面」，不影响开合
    lastMinWidth = -1
  }
}

/** 按当前状态重算窗口最小宽度（幂等，可在任意时机调用）。 */
function syncWindowMinWidth() {
  const w = agentPanelVisible.value
    ? agentWindowMinWidth(mainWidth.value)
    : MAIN_MIN_WIDTH
  void setWindowMinWidth(getCurrentWindow(), w)
}

/** 窗口右侧到显示器边缘的可用逻辑宽度；拿不到显示器信息时视为不限。 */
async function rightFreeSpace(win: ReturnType<typeof getCurrentWindow>): Promise<number> {
  try {
    const monitor = await currentMonitor()
    if (!monitor) return Number.POSITIVE_INFINITY
    const scale = monitor.scaleFactor || 1
    const pos = await win.outerPosition()
    const size = await win.outerSize()
    const monitorRight = (monitor.position.x + monitor.size.width) / scale
    const windowRight = (pos.x + size.width) / scale
    return Math.max(0, monitorRight - windowRight)
  } catch {
    return Number.POSITIVE_INFINITY
  }
}

// 打开（或聚焦）右侧工作区；预置实体经 seed 直接注入（同一窗口，无事件桥）
async function openAgentWorkspace(seed?: AgentWorkspaceSeed) {
  agentPanelSeed.value = seed ?? null
  if (agentPanelOpen.value || panelBusy) return
  panelBusy = true
  try {
    // 读取上次关闭时的面板宽度（未设置/失败回退默认 440）
    const panelTarget = Math.max(await loadSavedPanelWidth(), AGENT_PANEL_MIN_WIDTH)
    const win = getCurrentWindow()
    const size = await win.innerSize() // 物理像素（inner，与 setSize 口径一致）
    // 缩放用窗口 scaleFactor（与 setSize 内部换算一致）；window.devicePixelRatio
    // 在高 DPI 环境可能与窗口缩放不同步，导致尺寸换算失真。
    const scale = await win.scaleFactor()
    const windowW = size.width / scale
    const height = size.height / scale
    // 主界面当前布局宽度（CSS px）；font_scale 会页面缩放，CSS px 与窗口逻辑 px 不等值
    const layoutW = window.innerWidth
    // 逻辑 px → CSS px 的换算比（缩放 100% 时为 1）
    const pxPerLogical = windowW > 0 ? layoutW / windowW : 1
    let grow = 0
    if (!(await win.isMaximized())) {
      // 窗口加宽「面板 + 分隔线」，按屏幕剩余空间截断
      grow = Math.max(
        0,
        Math.min(panelTarget + AGENT_DIVIDER_WIDTH, await rightFreeSpace(win)),
      )
      if (grow > 0) await win.setSize(new LogicalSize(windowW + grow, height))
    }
    // 面板优先拿回记住的宽度，不够的部分才从主界面让位（贴屏幕右边缘 / 最大化）
    const availableW = layoutW + grow * pxPerLogical
    mainWidth.value = clampMainWidth(availableW - panelTarget - AGENT_DIVIDER_WIDTH, availableW)
    viewportWidth.value = availableW
    // 展开期间抬高窗口最小宽度：主界面列宽不会被窗口边缘拖拽压缩
    await setWindowMinWidth(win, agentWindowMinWidth(mainWidth.value))
    agentPanelOpen.value = true
  } catch (e) {
    showToast(String(e))
  } finally {
    panelBusy = false
  }
}

async function toggleAgentWorkspace() {
  if (agentPanelOpen.value) {
    track('release.collapse_agent_workspace')
    await closeAgentPanel()
  } else {
    track('release.open_agent_workspace')
    await openAgentWorkspace()
  }
}

async function closeAgentPanel() {
  if (panelBusy) return
  panelBusy = true
  // 面板宽（含展开期间被窗口边缘拖拽改变的部分）先落库：下次打开按它恢复
  cancelPanelWidthSave()
  persistPanelWidth(agentPanelWidth.value)
  agentPanelOpen.value = false
  try {
    const win = getCurrentWindow()
    // 先复位最小宽度：否则关闭后窗口仍被卡在展开期间的最小宽度
    await setWindowMinWidth(win, MAIN_MIN_WIDTH)
    // 最大化时边框不可拖、宽度调节无意义：保持最大化，不缩窄窗口
    if (await win.isMaximized()) return
    const scale = await win.scaleFactor()
    const size = await win.innerSize()
    // 收起后窗口宽度 = 主界面列宽：主界面宽度不因开关而变化
    const targetW = Math.max(mainWidth.value, MAIN_MIN_WIDTH)
    await win.setSize(new LogicalSize(targetW, size.height / scale))
  } catch {
    // 恢复尺寸失败不影响面板关闭
  } finally {
    panelBusy = false
  }
}

async function loadAgentConfig() {
  try {
    agentConfig.value = await getAgentConfig()
  } catch {
    agentConfig.value = null
  }
}

const countdown = ref('')
const releaseSearch = ref('')
const releaseStatusFilter = ref<'all' | 'unread' | 'read'>('all')
// ── 通知定位（focus-release）下钻：App 只保存目标与递增 token，
//    由 ReleaseTab 在每次“筛选/搜索/视图/数据就绪”后响应最新 token 执行定位。──
const focusTarget = ref<number | null>(null)
const focusReleaseToken = ref(0)
const polling = ref(false)
const sourceChecking = ref(false)
let countdownTimer: ReturnType<typeof setInterval> | null = null
let countdownSeconds = 0
let countdownReady = false

const unlisteners: (() => void)[] = []

const toastMessage = ref('')
const toastVisible = ref(false)
let toastTimer: ReturnType<typeof setTimeout> | null = null
// Toast 队列：新消息在当前消息显示期间进入队列，等旧消息消失后依次显示
const toastQueue: string[] = []
let toastSwapTimer: ReturnType<typeof setTimeout> | null = null
// 鼠标悬浮在 Toast 上时暂停消失计时。必须保留：错误详情、导出/备份路径这类内容
// 用户要移过去选中复制，没有暂停就选不完。
let toastHovered = false
// 需跟随 .toast-leave-active 的 --dur-overlay（220ms）+ 20ms 余量：早于离场动画换内容会看到新旧两条重叠
const TOAST_SWAP_DELAY = 240
const TOAST_MIN_DURATION = 2600
const TOAST_MAX_DURATION = 6000
// 按字数定停留时长：取中文默读约 5.7 字/秒 + 约 1.2s 反应余量，短按下限、超长封顶。
// 这只是"够读完"的基础时长，需要复制时靠悬浮暂停续命。
function toastDuration(message: string): number {
  const n = [...message].length
  return Math.min(TOAST_MAX_DURATION, Math.max(TOAST_MIN_DURATION, 1200 + n * 175))
}

// Toast 是视口 fixed 定位，Agent 面板打开时窗口右下角正好是面板的输入区（发送/附件按钮），
// 故面板可见时整块左移到面板左侧。不能用 pointer-events: none 穿透：按钮视觉上仍被盖住，
// 且 Toast 里的错误信息、导出路径会无法选中复制。面板宽度可拖拽，故宽度用响应式状态算而非硬编码。
const TOAST_GAP = 20
const agentPanelVisible = computed(() => agentPanelOpen.value && (agentConfig.value?.enabled ?? false))
const toastStyle = computed(() => {
  const right = agentPanelVisible.value
    ? agentPanelWidth.value + AGENT_DIVIDER_WIDTH + TOAST_GAP
    : TOAST_GAP
  return {
    right: `${right}px`,
    maxWidth: `min(360px, calc(100vw - ${right + TOAST_GAP}px))`,
  }
})

const selectionMenu = ref<{ x: number; y: number } | null>(null)
const inputContextMenu = ref<{ x: number; y: number; target: HTMLElement } | null>(null)

// ── 开发者统计面板（仅开发模式）──────────────────────
// Ctrl+Shift+U 呼出；动态 import 只在 DEV 分支执行，生产构建永不加载该模块。
const showStatsDev = ref(false)
const StatsDevPanelComp = shallowRef<Component | null>(null) as Ref<Component | null>

async function toggleStatsDev() {
  if (!import.meta.env.DEV) return
  if (!StatsDevPanelComp.value) {
    const mod = await import('./dev/StatsDevPanel.vue')
    StatsDevPanelComp.value = mod.default as Component
  }
  showStatsDev.value = !showStatsDev.value
}

function onGlobalKeydown(e: KeyboardEvent) {
  // 面板仅开发模式存在：非 DEV 直接返回，避免生产环境吞掉 Ctrl+Shift+U 却无动作
  if (!import.meta.env.DEV) return
  if (e.ctrlKey && e.shiftKey && (e.key === 'U' || e.key === 'u')) {
    e.preventDefault()
    void toggleStatsDev()
  }
}
const inputMenuItems = computed<ContextMenuItem[]>(() => [
  { id: 'cut', label: t('context.cut') },
  { id: 'copy', label: t('context.copy') },
  { id: 'paste', label: t('context.paste') },
  { id: 'selectAll', label: t('context.select_all') },
])
const selectionMenuItems = computed<ContextMenuItem[]>(() => [
  { id: 'copySelection', label: t('context.copy') },
])

function closeAllMenus() {
  selectionMenu.value = null
  inputContextMenu.value = null
}

async function handleCopySelection() {
  const text = window.getSelection()?.toString().trim()
  if (text) {
    try {
      await copyTextToClipboard(text)
    } catch (e: unknown) {
      showToast(t('release.copy_failed') + (e instanceof Error ? e.message : String(e)))
    }
  }
  closeAllMenus()
}

function handleSelectionMenuAction(actionId: string) {
  if (actionId === 'copySelection') {
    handleCopySelection()
  }
}

async function execInputAction(actionId: string) {
  const el = inputContextMenu.value?.target
  if (!el) return
  inputContextMenu.value = null
  el.focus()
  if (actionId === 'cut') {
    document.execCommand('cut')
  } else if (actionId === 'copy') {
    document.execCommand('copy')
  } else if (actionId === 'paste') {
    try {
      const text = await readText()
      document.execCommand('insertText', false, text)
    } catch {
      // 静默失败
    }
  } else if (actionId === 'selectAll') {
    document.execCommand('selectAll')
  }
}

function showToast(msg: string) {
  toastQueue.push(msg)
  // 当前无消息显示且无切换间隙时立即显示；否则排队等旧消息消失
  if (!toastVisible.value && !toastSwapTimer) {
    showNextToast()
  }
}

function showNextToast() {
  const next = toastQueue.shift()
  if (next === undefined) return
  toastMessage.value = next
  toastVisible.value = true
  // 鼠标正悬浮在旧消息上时不启动计时，等移开后由 handleToastMouseLeave 补启动
  if (!toastHovered) startToastTimer()
}

function startToastTimer() {
  clearToastTimer()
  toastTimer = setTimeout(() => {
    toastTimer = null
    dismissCurrentToast()
  }, toastDuration(toastMessage.value))
}

function clearToastTimer() {
  if (toastTimer) {
    clearTimeout(toastTimer)
    toastTimer = null
  }
}

function handleToastMouseEnter() {
  toastHovered = true
  clearToastTimer()
}

function handleToastMouseLeave() {
  toastHovered = false
  if (toastVisible.value) startToastTimer()
}

function dismissCurrentToast() {
  if (!toastVisible.value) return
  toastVisible.value = false
  // 等离场动画结束再显示队列中的下一条，避免新旧消息内容跳变
  toastSwapTimer = setTimeout(() => {
    toastSwapTimer = null
    showNextToast()
  }, TOAST_SWAP_DELAY)
}

provide(ShowToastKey, showToast)
// 全局错误兜底的 toast 出口：main.ts 的处理器不在组件树内，拿不到 provide/inject
setErrorToastSink(showToast)
provide(AiEnabledKey, computed(() => settings.value.deepseek_enabled && settings.value.deepseek_api_key_set))
// 「显示重要度」开关：仅控制版本卡片/详情/筛选的 UI 展示（设置页 AI 分组）
provide(ShowImportanceKey, computed(() => settings.value.show_importance))
// Agent 总开关：独立于 DeepSeek（本地 pi CLI 与在线 API 互不依赖）
provide(AgentEnabledKey, computed(() => agentConfig.value?.enabled ?? false))
provide(AgentWorkspaceKey, openAgentWorkspace)
provide(AgentPanelOpenKey, agentPanelOpen)
provide(AgentToggleKey, toggleAgentWorkspace)
// 诊断统计开关：跟随设置项启停（关闭时 track() no-op + 丢弃未上报计数）
watch(() => settings.value.enable_usage_stats, v => setUsageTrackingEnabled(v), { immediate: true })

// 设置页关掉 Agent 总开关时，面板组件被 v-if 卸载但 agentPanelOpen 仍为 true →
// 窗口保持加宽且无法经面板收回。监听配置变化，enabled 变 false 且面板开着时
// 自动收回（closeAgentPanel 内部有 panelBusy 防重入）。
watch(
  () => agentConfig.value?.enabled,
  (enabled) => {
    if (!enabled && agentPanelOpen.value) {
      void closeAgentPanel()
    }
  },
)

// 窗口最小宽度只跟「面板是否可见 + 主界面列宽」有关。immediate 让挂载时就把残留值
// 拉回 710——前端重载（HMR/整页刷新）会丢掉 agentPanelOpen，但 OS 的窗口最小宽度
// 是窗口实例级的，不在这里复位就会一直卡在展开期间的大值上。
watch(agentPanelVisible, (visible) => {
  syncWindowMinWidth()
  if (visible) return
  // 不可见但状态还说开着（如关面板时正有开关在飞），补齐关闭流程
  if (agentPanelOpen.value) void closeAgentPanel()
}, { immediate: true })

function repoKey(sourceType: string, owner: string, repo: string): string {
  return sourceRepoKey(sourceType, owner, repo)
}

function refreshLogs() {
  logRefreshKey.value++
}

const unreadReleaseCounts = computed<Record<string, number>>(() => {
  const counts: Record<string, number> = {}
  for (const release of releases.value) {
    if (!isUnreadStatus(release.notification_status, release.snooze_until)) continue
    const key = repoKey(release.source_type, release.owner, release.repo)
    counts[key] = (counts[key] || 0) + 1
  }
  return counts
})

const totalReleaseCounts = computed<Record<string, number>>(() => {
  const counts: Record<string, number> = {}
  for (const release of releases.value) {
    const key = repoKey(release.source_type, release.owner, release.repo)
    counts[key] = (counts[key] || 0) + 1
  }
  return counts
})

async function loadAll() {
  await Promise.allSettled([loadSources(), loadReleases(), loadSettings(), loadAgentConfig()])
}

async function loadSources() {
  sourcesLoading.value = true
  try {
    sources.value = await listSources()
    sourcesError.value = ''
  } catch (e: unknown) {
    const detail = e instanceof Error ? e.message : String(e)
    sourcesError.value = t('app.load_failed', detail)
    showToast(t('app.load_failed', detail))
  } finally {
    sourcesLoading.value = false
  }
}
async function loadReleases() {
  releasesLoading.value = true
  try {
    releases.value = await getReleaseCatalog()
    releasesError.value = ''
  } catch (e: unknown) {
    const detail = e instanceof Error ? e.message : String(e)
    releasesError.value = t('app.load_failed', detail)
    showToast(t('app.load_failed', detail))
  } finally {
    releasesLoading.value = false
  }
}

// ── 刷新合帧：组件 emit('update')（操作成功后就近刷新）与后端 release-state-changed
// 事件（托盘角标主消费，前端列表顺手监听）会在同一次操作中先后到达，双路径各拉
// 一次全量目录。50ms 窗口内合并成一次实际重拉，
// 首次加载/轮询完成等单路径刷新不受影响（直接调 loadReleases）。──
let releasesRefreshTimer: ReturnType<typeof setTimeout> | null = null
function scheduleLoadReleases() {
  if (releasesRefreshTimer !== null) return
  releasesRefreshTimer = setTimeout(() => {
    releasesRefreshTimer = null
    void loadReleases()
  }, 50)
}

/** 日志刷新合帧：refreshLogs 本身极轻（logRefreshKey++），但双路径到达两个 tick
 *  会各触发一次 LogTab 重拉，同样 50ms 合并。 */
let logsRefreshTimer: ReturnType<typeof setTimeout> | null = null
function scheduleRefreshLogs() {
  if (logsRefreshTimer !== null) return
  logsRefreshTimer = setTimeout(() => {
    logsRefreshTimer = null
    refreshLogs()
  }, 50)
}

async function loadSettings() {
  try {
    settings.value = await getSettings()
    setLocale(settings.value.language)
    applyTheme(settings.value.theme)
    applyFontScale(settings.value.font_scale)
  } catch (e: unknown) {
    showToast(t('app.load_failed', e instanceof Error ? e.message : String(e)))
  }
}

let systemThemeMedia: MediaQueryList | null = null
function watchSystemTheme() {
  if (systemThemeMedia) systemThemeMedia.onchange = null
  systemThemeMedia = window.matchMedia('(prefers-color-scheme: dark)')
  systemThemeMedia.onchange = () => {
    if (settings.value.theme === 'system') {
      applyTheme('system')
    }
  }
}

async function syncCountdown(refreshLogsOnJump = true) {
  const secs = await getPollCountdown()
  const prev = countdownSeconds
  countdownSeconds = secs
  countdown.value = formatCountdown(secs)
  if (refreshLogsOnJump && countdownReady && secs > prev + 30) {
    refreshLogs()
  }
  countdownReady = true
}

function startCountdown() {
  if (countdownTimer) clearInterval(countdownTimer)
  syncCountdown()
  countdownTimer = setInterval(() => {
    if (countdownSeconds > 0) {
      countdownSeconds--
      countdown.value = formatCountdown(countdownSeconds)
    }
    if (countdownSeconds <= 0) {
      syncCountdown()
    } else if (countdownSeconds % 60 === 0) {
      syncCountdown()
    }
  }, 1000)
}

async function handlePoll() {
  if (polling.value || sourceChecking.value) return
  track('app.check_now')
  const enabled = sources.value.filter(s => s.enabled)
  if (enabled.length === 0) {
    showToast(t('app.no_sources'))
    return
  }
  polling.value = true
  try {
    const result = await triggerPoll()
    if (result.new_releases.length === 0) {
      showToast(t('app.already_latest'))
    } else {
      showToast(t('app.new_found', String(result.new_releases.length)))
    }
  } catch (e: unknown) {
    showToast(t('app.check_failed', e instanceof Error ? e.message : String(e)))
  } finally {
    polling.value = false
  }
}

function handleSourceCheckResult(count: number) {
  if (count === 0) {
    showToast(t('app.already_latest'))
  } else {
    showToast(t('app.new_found', String(count)))
  }
}

function openSourceReleases(query: string) {
  releaseSearch.value = query
  releaseStatusFilter.value = 'all'
  activeTab.value = 'releases'
}

/** 点击通知主体：切到版本列表并定位到该条 release。
 *  源/重要度/旗标/版本等筛选与视图模式状态都持有在 ReleaseTab，「重置筛选 + 定位滚动」
 *  也在其中；App.vue 只负责下发 focusTarget（目标 id）+ 递增 focusReleaseToken，
 *  并监听 focus-consumed/focus-not-found 反馈——目标缺失 / 列表视图下找不到时
 *  以 Toast 提示。同仓库存在多条 release 时能唯一定位到被点击的那一条。 */
function focusReleaseById(id: number) {
  focusTarget.value = id
  focusReleaseToken.value++
  activeTab.value = 'releases'
}

function handleFocusConsumed() {
  // ReleaseTab 只对最新一次目标上报 consumed，可直接清空
  focusTarget.value = null
}

function handleFocusNotFound() {
  if (focusTarget.value === null) return
  // ReleaseTab 确认目标在已加载数据中不存在（被删除等）：提示用户，避免静默失败
  focusTarget.value = null
  showToast(t('release.focus_missing'))
}

function openSourceUnreadReleases(query: string) {
  releaseSearch.value = query
  releaseStatusFilter.value = 'unread'
  activeTab.value = 'releases'
}

useEscapeToTray(computed(() => settings.value.minimize_to_tray))
// 窗口隐藏/失焦后清空瞬态悬浮层并进入失效期：Chromium 在隐藏时不补 mouseleave、
// 重新显示时又会重放隐藏前的 focus（详见 useTransientUiGuard）；
// 顺带把 Toast 的悬浮暂停解除——隐藏时不会再有 mouseleave，
// 计时器会永久停摆，窗口再出现时 Toast 就一直挂在右下角。
useTransientUiGuard({
  onInvalidate: () => {
    if (toastHovered) handleToastMouseLeave()
  },
})
// 外链一律交给系统浏览器，webview 自身永不导航
useExternalLinkGuard()

onMounted(async () => {
  syncSourceCapabilities() // 源类型能力位以后端 list_source_types 为权威（不阻塞，失败静默降级）
  await loadAll()
  watchSystemTheme()
  startCountdown()

  // 视口宽跟随窗口边缘拖拽：面板宽度、Toast 偏移都靠它算
  window.addEventListener('resize', onViewportResize)
  unlisteners.push(() => window.removeEventListener('resize', onViewportResize))

  const handleContextMenu = (e: MouseEvent) => {
    const target = e.target as HTMLElement
    const tag = target.tagName
    // 输入元素：显示自定义剪切/复制/粘贴/全选菜单
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable) {
      e.preventDefault()
      closeAllContextMenus()
      inputContextMenu.value = { x: e.clientX, y: e.clientY, target }
      return
    }
    const selection = window.getSelection()
    const selected = selection && selection.toString().trim()
    if (selected) {
      e.preventDefault()
      closeAllContextMenus()
      selectionMenu.value = { x: e.clientX, y: e.clientY }
      return
    }
    e.preventDefault()
  }
  document.addEventListener('contextmenu', handleContextMenu)
  unlisteners.push(() => document.removeEventListener('contextmenu', handleContextMenu))

  registerCloser(closeAllMenus)
  document.addEventListener('click', closeAllMenus)
  unlisteners.push(() => {
    unregisterCloser(closeAllMenus)
    document.removeEventListener('click', closeAllMenus)
  })

  // 开发者统计面板快捷键仅 DEV 注册：生产包完全不包含面板逻辑入口
  if (import.meta.env.DEV) {
    window.addEventListener('keydown', onGlobalKeydown)
    unlisteners.push(() => window.removeEventListener('keydown', onGlobalKeydown))
  }

  const navigateUnlisten = await events.navigate.listen((event) => {
    if (event.payload === 'sources' || event.payload === 'releases' || event.payload === 'settings') {
      activeTab.value = event.payload as 'sources' | 'releases' | 'logs' | 'settings'
    }
  })
  unlisteners.push(navigateUnlisten)

  // 点击桌面通知主体（非按钮）：定位到该条 release。
  // 注册在 loadAll() 之后，此时 releases 已就绪，正常情况下能命中
  const focusUnlisten = await events.focusRelease.listen((event) => {
    focusReleaseById(event.payload)
  })
  unlisteners.push(focusUnlisten)

  const pollUnlisten = await events.pollCompleted.listen(() => {
    loadSources()
    loadReleases()
    refreshLogs()
    syncCountdown(false)
  })
  unlisteners.push(pollUnlisten)

  const stateUnlisten = await events.releaseStateChanged.listen(() => {
    scheduleLoadReleases()
    scheduleRefreshLogs()
  })
  unlisteners.push(stateUnlisten)

  // 后台 AI 批（摘要补全/翻译）收尾时写日志：它是 fire-and-forget，可能远晚于
  // 发起它的那轮轮询才结束，届时已无人触发日志刷新。
  const logAppendedUnlisten = await events.logAppended.listen(() => {
    scheduleRefreshLogs()
  })
  unlisteners.push(logAppendedUnlisten)

  const autoDisabledUnlisten = await events.sourceAutoDisabled.listen((event) => {
    showToast(t('app.source_auto_disabled', event.payload.owner, event.payload.repo, String(event.payload.failures)))
    loadSources()
    refreshLogs()
  })
  unlisteners.push(autoDisabledUnlisten)
})

onUnmounted(() => {
  for (const unlisten of unlisteners) {
    unlisten()
  }
  unlisteners.length = 0
  if (panelWidthSaveTimer) {
    clearTimeout(panelWidthSaveTimer)
    panelWidthSaveTimer = null
  }
  // 刷新合帧定时器（同上，防卸载后回调修改已销毁组件的 ref）
  if (releasesRefreshTimer !== null) {
    clearTimeout(releasesRefreshTimer)
    releasesRefreshTimer = null
  }
  if (logsRefreshTimer !== null) {
    clearTimeout(logsRefreshTimer)
    logsRefreshTimer = null
  }
  // 清理定时器与媒体监听，避免卸载后回调修改已销毁组件的 ref（HMR/测试场景）
  if (countdownTimer) {
    clearInterval(countdownTimer)
    countdownTimer = null
  }
  if (toastTimer) {
    clearTimeout(toastTimer)
    toastTimer = null
  }
  if (toastSwapTimer) {
    clearTimeout(toastSwapTimer)
    toastSwapTimer = null
  }
  if (systemThemeMedia) {
    systemThemeMedia.onchange = null
  }
  // 冲刷未上报的点击统计（卸载前最后一批）
  void flushUsageTrackingNow()
})
</script>

<template>
  <div class="app-shell">
    <div
      class="app-main-col"
      :style="agentPanelVisible ? { flex: '0 1 auto', width: mainWidth + 'px' } : undefined"
    >
      <div class="app">
        <header class="app-header">
      <div class="header-top">
        <h1>{{ t('app.title') }}</h1>
        <div class="header-top-actions">
          <button
            v-if="agentConfig?.enabled"
            class="release-agent-btn"
            :class="{ open: agentPanelOpen }"
            :title="agentPanelOpen ? t('agent.collapse_workspace') : t('agent.expand_workspace')"
            @click="toggleAgentWorkspace"
          >
            <svg class="release-agent-btn-icon"><use href="/icons.svg#agent-icon"/></svg>
            <svg class="release-agent-btn-arrow"><use :href="agentPanelOpen ? '/icons.svg#chevron-left-icon' : '/icons.svg#chevron-right-icon'"/></svg>
          </button>
          <!-- 分组分隔线：Agent 面板开关与「立即检查」主操作隔开，降低误触 -->
          <span v-if="agentConfig?.enabled" class="header-actions-divider"></span>
          <button class="btn-primary" :disabled="polling || sourceChecking" @click="handlePoll">
            {{ polling || sourceChecking ? t('app.checking') : t('app.check_now') }}
          </button>
        </div>
      </div>
      <div class="header-bottom">
        <nav class="tabs">
          <button :class="{ active: activeTab === 'sources' }" @click="activeTab = 'sources'"><svg class="tab-icon"><use href="/icons.svg#sources-icon"/></svg>{{ t('tab.sources') }}</button>
          <button :class="{ active: activeTab === 'releases' }" @click="activeTab = 'releases'"><svg class="tab-icon"><use href="/icons.svg#release-icon"/></svg>{{ t('tab.releases') }}</button>
          <button :class="{ active: activeTab === 'logs' }" @click="activeTab = 'logs'"><svg class="tab-icon"><use href="/icons.svg#log-icon"/></svg>{{ t('tab.logs') }}</button>
          <button :class="{ active: activeTab === 'settings' }" @click="activeTab = 'settings'"><svg class="tab-icon"><use href="/icons.svg#settings-icon"/></svg>{{ t('tab.settings') }}</button>
        </nav>
        <span class="countdown-text" v-if="countdown">{{ t('app.next_check') }}{{ countdown }}</span>
      </div>
    </header>

    <!-- v-show 直接绑在组件 vnode 上时，Vue 的 shouldUpdateComponent 见到
         nextVNode.dirs 就无条件返回 true，父级每次重渲染都把三个隐藏 tab 整棵子树
         重算一遍（打字时改的是 App 里的 releaseSearch，故每键都触发）。
         指令挪到包裹 div 上，组件回到 props 浅比较，隐藏 tab 不再被无关状态牵连。 -->
    <main class="app-main" :class="{ 'is-scrolled': mainScrolled }" @scroll.passive="onMainScroll">
      <div v-show="activeTab === 'sources'">
        <SourceTab :sources="sources" :polling="polling || sourceChecking" :unread-release-counts="unreadReleaseCounts" :total-release-counts="totalReleaseCounts" :show-source-type-icons="settings.show_source_type_icons"
          :loading="sourcesLoading" :load-error="sourcesError"
          @update="loadSources(); scheduleLoadReleases(); scheduleRefreshLogs()"
          @check-result="handleSourceCheckResult"
          @check-busy="sourceChecking = $event"
          @open-releases="openSourceReleases"
          @open-unread-releases="openSourceUnreadReleases" />
      </div>
      <div v-show="activeTab === 'releases'">
        <ReleaseTab v-model:search="releaseSearch" v-model:statusFilter="releaseStatusFilter" :releases="releases" :focus-target="focusTarget" :focus-token="focusReleaseToken" :loading="releasesLoading" :load-error="releasesError" @focus-consumed="handleFocusConsumed" @focus-not-found="handleFocusNotFound" @reload="loadReleases" @update="scheduleLoadReleases(); scheduleRefreshLogs()" />
      </div>
      <div v-show="activeTab === 'logs'">
        <LogTab :refresh-key="logRefreshKey" @update="refreshLogs()" />
      </div>
      <div v-show="activeTab === 'settings'">
        <SettingsTab :settings="settings"
          @update="(pollChanged, forceReload) => { loadSettings(); if (pollChanged) startCountdown(); if (forceReload) { loadSources(); loadReleases(); } refreshLogs(); applyTheme(settings.theme) }"
          @agent-config-changed="loadAgentConfig()"
          @update-log-written="scheduleRefreshLogs()" />
      </div>
    </main>

    <Transition name="toast">
      <div
        v-if="toastVisible"
        class="toast"
        :style="toastStyle"
        role="status"
        aria-live="polite"
        @mouseenter="handleToastMouseEnter"
        @mouseleave="handleToastMouseLeave"
      >{{ toastMessage }}</div>
    </Transition>

    <ContextMenu v-if="selectionMenu" :x="selectionMenu.x" :y="selectionMenu.y" :items="selectionMenuItems" @action="handleSelectionMenuAction" @close="selectionMenu = null" />
    <ContextMenu v-if="inputContextMenu" :x="inputContextMenu.x" :y="inputContextMenu.y" :items="inputMenuItems" @action="execInputAction" @close="inputContextMenu = null" />
    <StatsDevPanelComp v-if="showStatsDev && StatsDevPanelComp" @close="showStatsDev = false" />
      </div>
    </div>
    <div
      v-if="agentPanelVisible"
      class="agent-divider"
      title="拖拽调节主界面宽度"
      @mousedown="onDividerMouseDown"
    ></div>
    <AgentWorkspace v-if="agentPanelVisible" :seed="agentPanelSeed" :width="agentPanelWidth" @close="closeAgentPanel" />
  </div>
</template>

<style scoped>
/* 壳布局：主界面列 + 右侧 Agent 工作区列，两列各自独立滚动 */
.app-shell {
  height: 100vh;
  display: flex;
  overflow: hidden;
}
/* 面板关闭时主界面占满窗口；打开时列宽由 mainWidth 内联给定，窗口边缘拖拽的
   增量全部落到 Agent 面板。shrink 保留 1 —— 窗口窄到放不下时先让主界面收缩，
   下限由 min-width（与 App.vue 的 MAIN_MIN_WIDTH 同步）兜底 */
.app-main-col {
  flex: 1 1 auto;
  min-width: 710px;
  display: flex;
  overflow: hidden;
}
.app-main-col .app {
  flex: 1;
  min-width: 0;
  height: 100%;
}

/* 标题栏右侧操作组：Agent 开合按钮 + 分隔线 + 立即检查（gap 14px 拉开两组，防误触） */
.header-top-actions {
  display: flex;
  align-items: center;
  gap: 14px;
}

.header-actions-divider {
  width: 1px;
  height: 16px;
  background: var(--border);
  flex-shrink: 0;
}

/* 分隔线：主界面与 Agent 工作区之间，可拖拽调节两边宽度 */
.agent-divider {
  flex: 0 0 5px;
  cursor: col-resize;
  background: var(--border);
  transition: background var(--dur-state) var(--ease-out);
  position: relative;
  z-index: var(--z-sticky);
}
.agent-divider:hover,
.agent-divider.active {
  background: var(--primary);
}
.agent-divider::after {
  content: '';
  position: absolute;
  inset: 0 -3px;
}

.toast {
  position: fixed;
  bottom: 20px;
  z-index: var(--z-toast);
  padding: 9px 18px;
  background: var(--ink);
  color: var(--on-ink);
  border-radius: var(--radius);
  font-size: var(--fs-body);
  box-shadow: var(--shadow-lg);
  /* right / max-width 由 toastStyle 绑定：面板打开时整块左移到面板左侧，
     避开面板底部的发送/附件按钮（详见 toastStyle 注释）。
     这里刻意不设 pointer-events: none——那会让 Toast 里的错误详情、导出
     路径无法选中复制，且按钮视觉上仍被盖住，只是把「点不动」换成了「看不见」。 */
  overflow-wrap: anywhere;
  /* 错误详情等需要复制；拖拽分隔线时 style.css 的 !important 仍会压过这里 */
  user-select: text;
  -webkit-user-select: text;
}

.toast-enter-active {
  transition: all var(--dur-overlay) var(--ease-out);
}

.toast-leave-active {
  transition: all var(--dur-overlay) var(--ease-out);
}

.toast-enter-from {
  opacity: 0;
  transform: translateX(60px);
}

.toast-leave-to {
  opacity: 0;
  transform: translateX(60px);
}

</style>
