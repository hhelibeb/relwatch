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
// Agent 工作区当前宽度（逻辑 px）：拖窗口右边框调节，持久化到 app_settings
const agentPanelWidth = ref(AGENT_PANEL_DEFAULT_WIDTH)
// 开关互斥锁：innerSize/setSize 是异步的，防快速连点导致交错竞态
let panelBusy = false

// ── 窗口边框拖拽跟踪（面板展开期间生效）──
// 目标：拖窗口右边框 → 调 Agent 工作区宽度（主界面宽度不变）；
//       拖窗口左边框 → 调主界面宽度（Agent 宽度不变）。
// 判定依据：拖右边框时窗口左上角 x 不变、宽度变；拖左边框时 x 与宽度同步变化。
// 基准为逻辑像素（innerWidth + screenX）快照，每帧滚动更新；跨 DPI 场景下
// 逻辑宽度不变时仅刷新基准不误判。
let panelResizeBase: { w: number; x: number } | null = null
let panelWidthSaveTimer: ReturnType<typeof setTimeout> | null = null
// 面板宽度追踪改为 DOM resize（同步、零 IPC）：窗口尺寸与位置（screenX）在事件
// 派发时已是新值，面板宽度与视口宽度同帧生效，flex 布局不再出现
// 「主界面先宽后缩」的中间态。相比 Tauri onResized（IPC 异步派发 + 快照读取），
// 消除了更新滞后导致的抖动。
let panelDomResizeHandler: (() => void) | null = null

function clampPanelWidth(w: number, windowW: number): number {
  // 下限保证可读性；上限为主界面最小宽度之外的剩余空间（主界面永不被挤爆）
  const maxW = Math.max(AGENT_PANEL_MIN_WIDTH, windowW - MAIN_MIN_WIDTH)
  return Math.round(Math.min(Math.max(w, AGENT_PANEL_MIN_WIDTH), maxW))
}

function startPanelResizeTracking() {
  stopPanelResizeTracking()
  // 程序性 setSize（open 加宽窗口）的 resize 余波可能在本函数之后才派发：
  // 首个 resize 事件只校准基准不参与判定，避免把程序性加宽误判为用户拖拽
  let suppressFirst = true
  panelResizeBase = { w: window.innerWidth, x: window.screenX }
  panelDomResizeHandler = () => {
    if (suppressFirst) {
      suppressFirst = false
      panelResizeBase = { w: window.innerWidth, x: window.screenX }
      return
    }
    const base = panelResizeBase
    if (!base) return
    const newW = window.innerWidth // 逻辑 px（CSS），与 setSize/面板宽度同口径
    const newX = window.screenX // 逻辑 px 窗口位置：拖右边框时不变，拖左边框时同步变化
    const deltaW = newW - base.w
    const deltaX = newX - base.x
    if (Math.abs(deltaW) >= 0.5 && Math.abs(deltaX) <= 2) {
      // 右边框拖动：宽度变化全部归 Agent 面板，主界面宽度不变
      const next = clampPanelWidth(agentPanelWidth.value + deltaW, newW)
      if (next !== agentPanelWidth.value) {
        agentPanelWidth.value = next
        schedulePanelWidthSave()
      }
    } else if (Math.abs(deltaW) >= 0.5) {
      // 左边框拖动：主界面吸收宽度变化；窗口收窄到主界面下限（710）后，
      // 面板同步收缩，保持状态与 flex 实际显示一致（避免拉宽时跳变）
      if (newW - agentPanelWidth.value < MAIN_MIN_WIDTH) {
        const next = Math.max(newW - MAIN_MIN_WIDTH, AGENT_PANEL_MIN_WIDTH)
        if (next !== agentPanelWidth.value) {
          agentPanelWidth.value = next
          schedulePanelWidthSave()
        }
      }
    }
    // 纯平移 / 缩放变化：Agent 宽度不动，仅刷新基准
    panelResizeBase = { w: newW, x: newX }
  }
  window.addEventListener('resize', panelDomResizeHandler)
}

function stopPanelResizeTracking() {
  if (panelDomResizeHandler) {
    window.removeEventListener('resize', panelDomResizeHandler)
    panelDomResizeHandler = null
  }
  panelResizeBase = null
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
    // 窗口将同步加宽到「当前主界面 + 面板」：clamp 上限按加宽后的窗口计算，
    // 保存的宽度可完整恢复（主界面仍保 710 下限）；若按当前窗口宽（未含面板）
    // 计算，窄窗口会把恢复宽度错误压缩到主界面下限附近（面板越收越窄）。
    const windowW = size.width / scale
    agentPanelWidth.value = clampPanelWidth(agentPanelWidth.value, windowW + agentPanelWidth.value)
    await win.setSize(new LogicalSize(windowW + agentPanelWidth.value, size.height / scale))
    agentPanelOpen.value = true
    await startPanelResizeTracking()
  } catch (e) {
    showToast(String(e))
  } finally {
    panelBusy = false
  }
}

// 切换开合：ReleaseTab 工具栏按钮用（已打开则收回，未打开则展开）
async function toggleAgentWorkspace() {
  if (agentPanelOpen.value) {
    await closeAgentPanel()
  } else {
    await openAgentWorkspace()
  }
}

async function closeAgentPanel() {
  if (panelBusy) return
  panelBusy = true
  stopPanelResizeTracking()
  agentPanelOpen.value = false
  try {
    const win = getCurrentWindow()
    // 最大化时边框不可拖、宽度调节无意义：保持最大化，不缩窄窗口
    if (await win.isMaximized()) return
    const scale = await win.scaleFactor()
    const size = await win.innerSize()
    // 窗口缩窄 Agent 面板宽度 → 主界面宽度保持不变
    // （面板展开期间拖左边框调出的主界面宽度不被撤销）
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
let toastHovered = false
const TOAST_DURATION = 3000
// 与 .toast-leave-active 过渡时长（0.3s）对齐，离场动画结束后再显示下一条
const TOAST_SWAP_DELAY = 350

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
  }, TOAST_DURATION)
}

function clearToastTimer() {
  if (toastTimer) {
    clearTimeout(toastTimer)
    toastTimer = null
  }
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

function handleToastMouseEnter() {
  toastHovered = true
  clearToastTimer()
}

function handleToastMouseLeave() {
  toastHovered = false
  if (toastVisible.value) startToastTimer()
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

  const pollUnlisten = await events.pollCompleted.listen(() => {
    loadSources()
    loadReleases()
    refreshLogs()
    syncCountdown(false)
  })
  unlisteners.push(pollUnlisten)

  const stateUnlisten = await events.releaseStateChanged.listen(() => {
    loadReleases()
    refreshLogs()
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
  // 面板展开期间的边框拖拽跟踪与宽度保存定时器
  stopPanelResizeTracking()
  if (panelWidthSaveTimer) {
    clearTimeout(panelWidthSaveTimer)
    panelWidthSaveTimer = null
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
        <button class="btn-primary" :disabled="polling || sourceChecking" @click="handlePoll">
          {{ polling || sourceChecking ? t('app.checking') : t('app.check_now') }}
        </button>
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
        @update="loadSources(); loadReleases(); refreshLogs()"
        @check-result="handleSourceCheckResult"
        @check-busy="sourceChecking = $event"
        @open-releases="openSourceReleases"
        @open-unread-releases="openSourceUnreadReleases" />
      <ReleaseTab v-show="activeTab === 'releases'" v-model:search="releaseSearch" v-model:statusFilter="releaseStatusFilter" :releases="releases" @update="loadReleases(); refreshLogs()" />
      <LogTab v-show="activeTab === 'logs'" :refresh-key="logRefreshKey" @update="refreshLogs()" />
      <SettingsTab v-show="activeTab === 'settings'" :settings="settings"
        @update="(pollChanged, forceReload) => { loadSettings(); if (pollChanged) startCountdown(); if (forceReload) { loadSources(); loadReleases(); } refreshLogs(); applyTheme(settings.theme) }"
        @agent-config-changed="loadAgentConfig()" />
    </main>

    <Transition name="toast">
      <div v-if="toastVisible" class="toast" @mouseenter="handleToastMouseEnter" @mouseleave="handleToastMouseLeave">{{ toastMessage }}</div>
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
  right: 20px;
  bottom: 20px;
  z-index: 9999;
  padding: 9px 18px;
  background: var(--ink);
  color: var(--on-ink);
  border-radius: var(--radius);
  font-size: 13px;
  box-shadow: var(--shadow-lg);
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
