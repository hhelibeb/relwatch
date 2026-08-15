<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, provide, watch, shallowRef, type Component, type Ref } from 'vue'
import { ShowToastKey, AiEnabledKey, AgentEnabledKey, AgentWorkspaceKey, AgentPanelOpenKey, AgentToggleKey, type AgentWorkspaceSeed } from './injection-keys'
import ContextMenu, { type ContextMenuItem } from './components/common/ContextMenu.vue'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { events } from './bindings'
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
const AGENT_PANEL_WIDTH = 440
const agentPanelOpen = ref(false)
const agentPanelSeed = ref<AgentWorkspaceSeed | null>(null)
// 打开面板前的窗口尺寸（物理 px，inner 口径）：宽高都记录，收起时完整恢复，
// 保证多次开关循环后窗口尺寸不变。注意必须用 innerSize：setSize 设置的是
// inner（内容区）尺寸，若用 outerSize 读数当 inner 目标，每次循环会把标题栏/边框
// 高度再叠加一遍，窗口逐次放大（历史 bug）。
let mainSizeBeforePanel: { width: number; height: number } | null = null
// 开关互斥锁：innerSize/setSize 是异步的，防快速连点导致交错竞态
let panelBusy = false

// 打开（或聚焦）右侧工作区；预置实体经 seed 直接注入（同一窗口，无事件桥）
async function openAgentWorkspace(seed?: AgentWorkspaceSeed) {
  agentPanelSeed.value = seed ?? null
  if (agentPanelOpen.value || panelBusy) return
  panelBusy = true
  try {
    const win = getCurrentWindow()
    const size = await win.innerSize() // 物理像素（inner，与 setSize 口径一致）
    // 缩放用窗口 scaleFactor（与 setSize 内部换算一致）；window.devicePixelRatio
    // 在高 DPI 环境可能与窗口缩放不同步，导致尺寸换算失真。
    const scale = await win.scaleFactor()
    mainSizeBeforePanel = { width: size.width, height: size.height }
    await win.setSize(new LogicalSize(size.width / scale + AGENT_PANEL_WIDTH, size.height / scale))
    agentPanelOpen.value = true
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
  agentPanelOpen.value = false
  const saved = mainSizeBeforePanel
  mainSizeBeforePanel = null
  if (saved && saved.width > 0) {
    try {
      const win = getCurrentWindow()
      const scale = await win.scaleFactor()
      // 恢复展开前的完整 inner 尺寸（宽+高）；不读当前尺寸，展开中被放大的部分必须还原
      await win.setSize(new LogicalSize(saved.width / scale, saved.height / scale))
    } catch {
      // 恢复尺寸失败不影响面板关闭
    }
  }
  panelBusy = false
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
    <AgentWorkspace v-if="agentPanelOpen && agentConfig?.enabled" :seed="agentPanelSeed" @close="closeAgentPanel" />
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
