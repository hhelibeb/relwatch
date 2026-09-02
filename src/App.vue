<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, provide, watch, shallowRef, type Component, type Ref } from 'vue'
import { ShowToastKey, AiEnabledKey, AgentEnabledKey, AgentWorkspaceKey, AgentPanelOpenKey, AgentToggleKey, type AgentWorkspaceSeed } from './injection-keys'
import ContextMenu, { type ContextMenuItem } from './components/common/ContextMenu.vue'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { events, commands } from './bindings'
import { type Source, listSources, sourceRepoKey, syncSourceCapabilities } from './api/sources'
import { type ReleaseInfo, triggerPoll, getPollCountdown, getReleases } from './api/releases'
import { type AppSettings, getSettings, DEFAULT_SETTINGS } from './api/settings'
import { t, setLocale } from './i18n'
import { registerCloser, unregisterCloser, closeAllContextMenus } from './composables/contextMenuBus'
import { useEscapeToTray } from './composables/useEscapeToTray'
import { useExternalLinkGuard } from './composables/useExternalLinkGuard'
import { applyTheme } from './composables/useTheme'
import { setUsageTrackingEnabled, flushUsageTrackingNow, track } from './composables/useUsageTracking'
import { isUnreadStatus, formatCountdown } from './utils'
import SourceTab from './components/SourceTab.vue'
import ReleaseTab from './components/ReleaseTab.vue'
import LogTab from './components/LogTab.vue'
import SettingsTab from './components/SettingsTab.vue'
import AgentWorkspace from './components/AgentWorkspace.vue'
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
import { getAgentConfig, type AgentConfig } from './api/agent'

const activeTab = ref<'sources' | 'releases' | 'logs' | 'settings'>('sources')
const mainScrolled = ref(false)

function onMainScroll(e: Event) {
  const el = e.currentTarget as HTMLElement
  mainScrolled.value = el.scrollTop > 0
}

const sources = ref<Source[]>([])
const releases = ref<ReleaseInfo[]>([])
const logRefreshKey = ref(0)
const settings = ref<AppSettings>({ ...DEFAULT_SETTINGS })
// Agent 全局配置（独立于 AppSettings：含 JSON 数组字段，不走设置注册表）
const agentConfig = ref<AgentConfig | null>(null)

// ── Agent 工作区右栏：窗口整体加宽，主界面宽度不变，工作区占新增宽度 ──
const AGENT_PANEL_DEFAULT_WIDTH = 440
const AGENT_PANEL_MIN_WIDTH = 280
const MAIN_MIN_WIDTH = 710
const agentPanelOpen = ref(false)
const agentPanelSeed = ref<AgentWorkspaceSeed | null>(null)
// Agent 工作区当前宽度（逻辑 px）：分隔线拖拽调节，持久化到 app_settings
const agentPanelWidth = ref(AGENT_PANEL_DEFAULT_WIDTH)
// 本次打开是否加宽过窗口：仅加宽过的关闭时才缩窄，避免「打开没加宽、关闭却缩窄」错位
const panelWidened = ref(false)
// 开关互斥锁：innerSize/setSize 是异步的，防快速连点导致交错竞态
let panelBusy = false
let panelWidthSaveTimer: ReturnType<typeof setTimeout> | null = null

function clampPanelWidth(w: number, windowW: number): number {
  // 下限保证可读性；上限为主界面最小宽度之外的剩余空间（主界面永不被挤爆）
  const maxW = Math.max(AGENT_PANEL_MIN_WIDTH, windowW - MAIN_MIN_WIDTH)
  return Math.round(Math.min(Math.max(w, AGENT_PANEL_MIN_WIDTH), maxW))
}

function schedulePanelWidthSave() {
  if (panelWidthSaveTimer) clearTimeout(panelWidthSaveTimer)
  panelWidthSaveTimer = setTimeout(() => {
    panelWidthSaveTimer = null
    void commands.saveAgentWsWidth(agentPanelWidth.value).catch(() => {})
  }, 600)
}

// ── 分隔线拖拽：主界面与 Agent 工作区之间直接拖拽调节宽度（窗口总宽不变）──
let dividerDragging = false
let dividerStartX = 0
let dividerStartWidth = 0

function onDividerMouseDown(e: MouseEvent) {
  e.preventDefault()
  dividerDragging = true
  dividerStartX = e.clientX
  dividerStartWidth = agentPanelWidth.value
  document.addEventListener('mousemove', onDividerMouseMove)
  document.addEventListener('mouseup', onDividerMouseUp)
  document.body.classList.add('agent-resizing')
}

function onDividerMouseMove(e: MouseEvent) {
  if (!dividerDragging) return
  // 向左拖（clientX 减小）→ 面板变宽；向右拖 → 面板变窄
  const delta = dividerStartX - e.clientX
  const next = clampPanelWidth(dividerStartWidth + delta, window.innerWidth)
  if (next !== agentPanelWidth.value) {
    agentPanelWidth.value = next
  }
}

function onDividerMouseUp() {
  if (!dividerDragging) return
  dividerDragging = false
  document.removeEventListener('mousemove', onDividerMouseMove)
  document.removeEventListener('mouseup', onDividerMouseUp)
  document.body.classList.remove('agent-resizing')
  schedulePanelWidthSave()
}

// ── 宽度决策 ──
// 主界面内容在窗口内居中且最大 900px（.header-top/.header-bottom/.tab-content 的
// max-width:900px + margin:0 auto）。窗口足够宽时两侧本就有空白，面板直接在
// 内部弹出（不再加宽窗口）；不足时才加宽窗口。最大化场景（窗口尺寸固定）下无论
// 多宽都无需加宽，面板在窗口内挤压主界面（主界面有 710 下限保护）。
const mainMaxWidth = 900

/** 当前窗口宽度下，面板能否在内部弹出（≥900+面板宽+分隔线余量）。 */
function canFitPanelInside(windowW: number): boolean {
  return windowW >= mainMaxWidth + agentPanelWidth.value
}

// 打开（或聚焦）右侧工作区；预置实体经 seed 直接注入（同一窗口，无事件桥）
async function openAgentWorkspace(seed?: AgentWorkspaceSeed) {
  agentPanelSeed.value = seed ?? null
  if (agentPanelOpen.value || panelBusy) return
  panelBusy = true
  try {
    // 读取上次保存的面板宽度（未设置/失败回退默认 440）
    try {
      const saved = await commands.getAgentWsWidth()
      if (saved > 0) agentPanelWidth.value = saved
    } catch { /* 保持默认 */ }
    const win = getCurrentWindow()
    const size = await win.innerSize() // 物理像素（inner，与 setSize 口径一致）
    // 缩放用窗口 scaleFactor（与 setSize 内部换算一致）；window.devicePixelRatio
    // 在高 DPI 环境可能与窗口缩放不同步，导致尺寸换算失真。
    const scale = await win.scaleFactor()
    const windowW = size.width / scale
    panelWidened.value = false
    if (await win.isMaximized()) {
      // 最大化：窗口尺寸固定，只能压缩面板到窗口内可容纳的宽度（主界面保 710 下限）
      agentPanelWidth.value = clampPanelWidth(agentPanelWidth.value, windowW)
    } else if (canFitPanelInside(windowW)) {
      // 窗口足够宽：内部弹出，不加宽窗口
      agentPanelWidth.value = clampPanelWidth(agentPanelWidth.value, windowW)
    } else {
      // 窄窗口：加宽窗口，主界面宽度不变（clamp 上限按加宽后的窗口计算，
      // 保存的宽度可完整恢复，不被当前窗口宽压缩）
      const widenedW = windowW + agentPanelWidth.value
      agentPanelWidth.value = clampPanelWidth(agentPanelWidth.value, widenedW)
      await win.setSize(new LogicalSize(windowW + agentPanelWidth.value, size.height / scale))
      panelWidened.value = true
    }
    agentPanelOpen.value = true
  } catch (e) {
    showToast(String(e))
  } finally {
    panelBusy = false
  }
}

// 切换开合：标题栏按钮用（已打开则收回，未打开则展开）
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
  agentPanelOpen.value = false
  try {
    const win = getCurrentWindow()
    // 最大化时边框不可拖、宽度调节无意义：保持最大化，不缩窄窗口
    if (await win.isMaximized()) return
    // 仅本次打开加宽过窗口才缩窄；内部弹出（未加宽）时窗口尺寸不动
    if (!panelWidened.value) return
    const scale = await win.scaleFactor()
    const size = await win.innerSize()
    // 窗口缩窄 Agent 面板宽度 → 主界面宽度保持不变
    const targetW = Math.max(size.width / scale - agentPanelWidth.value, MAIN_MIN_WIDTH)
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
// 与 .toast-leave-active 过渡时长（0.3s）对齐，离场动画结束后再显示下一条
const TOAST_SWAP_DELAY = 350
const TOAST_MIN_DURATION = 2600
const TOAST_MAX_DURATION = 6000
// 按字数定停留时长：取中文默读约 5.7 字/秒 + 约 1.2s 反应余量，短按下限、超长封顶。
// 这只是"够读完"的基础时长，需要复制时靠悬浮暂停续命。
function toastDuration(message: string): number {
  const n = [...message].length
  return Math.min(TOAST_MAX_DURATION, Math.max(TOAST_MIN_DURATION, 1200 + n * 175))
}

// Toast 是视口 fixed 定位，Agent 面板打开时窗口右下角正好是面板的输入区（发送/附件按钮）。
// 曾尝试用 pointer-events: none 让点击穿透，但那是错的：按钮在视觉上仍被盖住，
// 用户得"越过一个看得见的浮层去点一个看不见的按钮"，而且 Toast 里的错误信息、
// 导出路径从此无法选中复制。正解是让 Toast 整块左移到面板左侧——既不压按钮，
// 又完整保留 hover 暂停与文本选择。面板宽度可拖拽，故用响应式状态算而非硬编码。
const TOAST_GAP = 20
const AGENT_DIVIDER_WIDTH = 5
const toastStyle = computed(() => {
  const panelVisible = agentPanelOpen.value && agentConfig.value?.enabled
  const right = panelVisible
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
  if (text) { try { await navigator.clipboard.writeText(text) } catch { /* ignore */ } }
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
provide(AiEnabledKey, computed(() => settings.value.deepseek_enabled && settings.value.deepseek_api_key_set))
// Agent 总开关：独立于 DeepSeek（本地 pi CLI 与在线 API 互不依赖）
provide(AgentEnabledKey, computed(() => agentConfig.value?.enabled ?? false))
provide(AgentWorkspaceKey, openAgentWorkspace)
provide(AgentPanelOpenKey, agentPanelOpen)
provide(AgentToggleKey, toggleAgentWorkspace)
// 诊断统计开关：跟随设置项启停（关闭时 track() no-op + 丢弃未上报计数）
watch(() => settings.value.enable_usage_stats, v => setUsageTrackingEnabled(v), { immediate: true })

// 面板悬空修复：设置页关掉 Agent 总开关时，面板组件被 v-if 卸载但 agentPanelOpen
// 仍为 true → 窗口保持加宽且无法经面板收回。监听配置变化，enabled 变 false 且
// 面板开着时自动收回（closeAgentPanel 内部有 panelBusy 防重入）。
watch(
  () => agentConfig.value?.enabled,
  (enabled) => {
    if (!enabled && agentPanelOpen.value) {
      void closeAgentPanel()
    }
  },
)

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
  try {
    sources.value = await listSources()
  } catch (e: unknown) {
    showToast(t('app.load_failed', e instanceof Error ? e.message : String(e)))
  }
}
async function loadReleases() {
  try {
    releases.value = await getReleases()
  } catch (e: unknown) {
    showToast(t('app.load_failed', e instanceof Error ? e.message : String(e)))
  }
}

// ── 刷新合帧：组件 emit('update')（操作成功后就近刷新）与后端 release-state-changed
// 事件（托盘角标主消费，前端列表顺手监听）会在同一次操作中先后到达，双路径各拉
// 一次全量（200 行含 body 全文列）。50ms 窗口内合并成一次实际重拉，
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

/** 点击通知主体：切到版本列表并回填搜索词过滤出该 release。
 *  同仓库存在多条 release 时不能精确到唯一条（搜索过滤方案固有，用户已接受此取舍）。 */
function focusReleaseById(id: number) {
  const release = releases.value.find(r => r.id === id)
  if (!release) {
    // release 尚未加载（冷启动竞态）或已被删除：仅切 tab，不做过滤
    activeTab.value = 'releases'
    return
  }
  // 视频源无 repo 语义，退化为按频道名（source_description）过滤；
  // releaseMatchesSearch 会匹配 source_description，故能命中
  const query = release.repo
    ? `${release.owner}/${release.repo}`
    : (release.source_description ?? release.owner)
  releaseSearch.value = query
  releaseStatusFilter.value = 'all'
  activeTab.value = 'releases'
}

function openSourceUnreadReleases(query: string) {
  releaseSearch.value = query
  releaseStatusFilter.value = 'unread'
  activeTab.value = 'releases'
}

useEscapeToTray(computed(() => settings.value.minimize_to_tray))
// 外链一律交给系统浏览器，webview 自身永不导航
useExternalLinkGuard()

onMounted(async () => {
  syncSourceCapabilities() // 源类型能力位以后端 list_source_types 为权威（不阻塞，失败静默降级）
  await loadAll()
  watchSystemTheme()
  startCountdown()

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
  // 宽度保存定时器
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
    <div class="app-main-col">
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

    <main class="app-main" :class="{ 'is-scrolled': mainScrolled }" @scroll.passive="onMainScroll">
      <SourceTab v-show="activeTab === 'sources'" :sources="sources" :polling="polling || sourceChecking" :unread-release-counts="unreadReleaseCounts" :total-release-counts="totalReleaseCounts" :show-source-type-icons="settings.show_source_type_icons"
        @update="loadSources(); scheduleLoadReleases(); scheduleRefreshLogs()"
        @check-result="handleSourceCheckResult"
        @check-busy="sourceChecking = $event"
        @open-releases="openSourceReleases"
        @open-unread-releases="openSourceUnreadReleases" />
      <ReleaseTab v-show="activeTab === 'releases'" v-model:search="releaseSearch" v-model:statusFilter="releaseStatusFilter" :releases="releases" @update="scheduleLoadReleases(); scheduleRefreshLogs()" />
      <LogTab v-show="activeTab === 'logs'" :refresh-key="logRefreshKey" @update="refreshLogs()" />
      <SettingsTab v-show="activeTab === 'settings'" :settings="settings"
        @update="(pollChanged, forceReload) => { loadSettings(); if (pollChanged) startCountdown(); if (forceReload) { loadSources(); loadReleases(); } refreshLogs(); applyTheme(settings.theme) }"
        @agent-config-changed="loadAgentConfig()" />
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
      v-if="agentPanelOpen && agentConfig?.enabled"
      class="agent-divider"
      title="拖拽调节宽度"
      @mousedown="onDividerMouseDown"
    ></div>
    <AgentWorkspace v-if="agentPanelOpen && agentConfig?.enabled" :seed="agentPanelSeed" :width="agentPanelWidth" @close="closeAgentPanel" />
  </div>
</template>

<style scoped>
/* 壳布局：主界面列 + 右侧 Agent 工作区列，两列各自独立滚动 */
.app-shell {
  height: 100vh;
  display: flex;
  overflow: hidden;
}
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

/* 两组操作间的竖直分隔线 */
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
  transition: background 0.15s ease;
  position: relative;
  z-index: 10;
}
.agent-divider:hover,
.agent-divider.active {
  background: var(--accent, #2e6fd0);
}
.agent-divider::after {
  content: '';
  position: absolute;
  inset: 0 -3px;
}

/* Toast */
.toast {
  position: fixed;
  bottom: 20px;
  z-index: 9999;
  padding: 9px 18px;
  background: var(--ink);
  color: var(--on-ink);
  border-radius: var(--radius);
  font-size: 13px;
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
  transition: all 0.35s cubic-bezier(0.16, 1, 0.3, 1);
}

.toast-leave-active {
  transition: all 0.3s ease-in;
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
