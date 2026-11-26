<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, provide, watch, shallowRef, nextTick, type Component, type Ref } from 'vue'
import { AppCatalogKey, ShowToastKey, AiEnabledKey, ShowImportanceKey, AgentEnabledKey, AgentWorkspaceKey, AgentPanelOpenKey, AgentToggleKey, ActiveTabKey, AutoReadDetailKey } from './injection-keys'
import ContextMenu, { type ContextMenuItem } from './components/common/ContextMenu.vue'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { events } from './bindings'
import { sourceRepoKey, syncSourceCapabilities } from './api/sources'
import { triggerPoll, getPollCountdown } from './api/releases'
import { copyTextToClipboard } from './api/client'
import { setErrorToastSink } from './api/report-error'
import { type AppSettings, getSettings, DEFAULT_SETTINGS } from './api/settings'
import { t, setLocale } from './i18n'
import { registerCloser, unregisterCloser, closeAllContextMenus, dispatchFind } from './composables/contextMenuBus'
import { useTransientUiGuard } from './composables/useTransientUiGuard'
import { useEscapeToTray } from './composables/useEscapeToTray'
import { useExternalLinkGuard } from './composables/useExternalLinkGuard'
import { applyTheme } from './composables/useTheme'
import { applyFontScale } from './composables/useFontScale'
import { setUsageTrackingEnabled, flushUsageTrackingNow, track } from './composables/useUsageTracking'
import { useAppCatalog } from './composables/useAppCatalog'
import { AGENT_DIVIDER_WIDTH, useAgentPanelLayout } from './composables/useAgentPanelLayout'
import { useToast } from './composables/useToast'
import { isUnreadStatus, formatCountdown, normalizeAutoReadDetail } from './utils'
import { isEditableTarget } from './utils/dom'
import { loadReleaseViewState } from './utils/releaseViewStorage'
import SourceTab from './components/SourceTab.vue'
import ReleaseTab from './components/ReleaseTab.vue'
import LogTab from './components/LogTab.vue'
import SettingsTab from './components/SettingsTab.vue'
import AgentWorkspace from './components/AgentWorkspace.vue'

const activeTab = ref<'sources' | 'releases' | 'logs' | 'settings'>('sources')
const mainScrolled = ref(false)

function onMainScroll(e: Event) {
  const el = e.currentTarget as HTMLElement
  mainScrolled.value = el.scrollTop > 0
}

const catalog = useAppCatalog((msg) => showToast(msg))
const {
  sources,
  releases,
  sourcesLoading,
  sourcesError,
  releasesLoading,
  releasesError,
  agentConfig,
  loadSources,
  loadReleases,
  loadAgentConfig,
  scheduleLoadReleases,
} = catalog
const logRefreshKey = ref(0)
const settings = ref<AppSettings>({ ...DEFAULT_SETTINGS })
const agentEnabled = computed(() => agentConfig.value?.enabled ?? false)

const {
  agentPanelOpen,
  agentPanelSeed,
  agentPanelWidth,
  agentPanelVisible,
  mainWidth,
  onDividerMouseDown,
  openAgentWorkspace,
  closeAgentPanel,
  toggleAgentWorkspace,
} = useAgentPanelLayout({
  agentEnabled,
  onError: (msg) => showToast(msg),
})

const {
  toastMessage,
  toastVisible,
  toastStyle,
  showToast,
  handleToastMouseEnter,
  handleToastMouseLeave,
  invalidateHover: invalidateToastHover,
} = useToast({
  agentPanelVisible,
  agentPanelWidth,
  dividerWidth: AGENT_DIVIDER_WIDTH,
})

const countdown = ref('')
const releaseSearch = ref('')
// 状态筛选的持久化由 ReleaseTab 统一写入，这里只在启动时读一次初值
const releaseStatusFilter = ref<'all' | 'unread' | 'read'>(loadReleaseViewState().statusFilter)
const releaseTab = ref<InstanceType<typeof ReleaseTab> | null>(null)
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

const TAB_KEYS: Record<string, 'sources' | 'releases' | 'logs' | 'settings'> = {
  '1': 'sources',
  '2': 'releases',
  '3': 'logs',
  '4': 'settings',
}

function onGlobalKeydown(e: KeyboardEvent) {
  if (e.isComposing) return
  // 面板仅开发模式存在：非 DEV 不吞 Ctrl+Shift+U
  if (import.meta.env.DEV && e.ctrlKey && e.shiftKey && (e.key === 'U' || e.key === 'u')) {
    e.preventDefault()
    void toggleStatsDev()
    return
  }
  if (!e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return
  const key = e.key.toLowerCase()
  if (key === 'f') {
    // WebView2 的 Ctrl+F 是浏览器查找栏，必须 preventDefault 才能接管
    e.preventDefault()
    if (dispatchFind()) return
    activeTab.value = 'releases'
    void nextTick(() => releaseTab.value?.focusSearch())
  } else if (key === 'r') {
    e.preventDefault()
    void handlePoll()
  } else if (TAB_KEYS[key]) {
    e.preventDefault()
    activeTab.value = TAB_KEYS[key]
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

provide(AppCatalogKey, catalog)
provide(ShowToastKey, showToast)
// 全局错误兜底的 toast 出口：main.ts 的处理器不在组件树内，拿不到 provide/inject
setErrorToastSink(showToast)
provide(AiEnabledKey, computed(() => settings.value.deepseek_enabled && settings.value.deepseek_api_key_set))
// 「显示重要度」开关：仅控制版本卡片/详情/筛选的 UI 展示（设置页 AI 分组）
provide(ShowImportanceKey, computed(() => settings.value.show_importance))
// 「打开详情时自动标为已读」档位：版本列表打开/翻页时按它标记（常规设置）
provide(AutoReadDetailKey, computed(() => normalizeAutoReadDetail(settings.value.auto_read_detail)))
// Agent 总开关：独立于 DeepSeek（本地 pi CLI 与在线 API 互不依赖）
provide(AgentEnabledKey, agentEnabled)
provide(AgentWorkspaceKey, openAgentWorkspace)
provide(AgentPanelOpenKey, agentPanelOpen)
provide(AgentToggleKey, toggleAgentWorkspace)
provide(ActiveTabKey, activeTab)
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
  onInvalidate: invalidateToastHover,
})
// 外链一律交给系统浏览器，webview 自身永不导航
useExternalLinkGuard()

onMounted(async () => {
  // 快捷键监听在 await 之前注册：卸载可能发生在异步体完成前，注册晚于卸载就没人移除它
  window.addEventListener('keydown', onGlobalKeydown)
  unlisteners.push(() => window.removeEventListener('keydown', onGlobalKeydown))

  syncSourceCapabilities() // 源类型能力位以后端 list_source_types 为权威（不阻塞，失败静默降级）
  await loadAll()
  watchSystemTheme()
  startCountdown()

  const handleContextMenu = (e: MouseEvent) => {
    const target = e.target as HTMLElement
    // 输入元素：显示自定义剪切/复制/粘贴/全选菜单
    if (isEditableTarget(target)) {
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
  if (logsRefreshTimer !== null) {
    clearTimeout(logsRefreshTimer)
    logsRefreshTimer = null
  }
  // 清理定时器与媒体监听，避免卸载后回调修改已销毁组件的 ref（HMR/测试场景）
  if (countdownTimer) {
    clearInterval(countdownTimer)
    countdownTimer = null
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
          <button class="btn-primary" :disabled="polling || sourceChecking" :title="t('app.check_now') + ' (Ctrl+R)'" @click="handlePoll">
            {{ polling || sourceChecking ? t('app.checking') : t('app.check_now') }}
          </button>
        </div>
      </div>
      <div class="header-bottom">
        <nav class="tabs">
          <button :class="{ active: activeTab === 'sources' }" :title="t('tab.sources') + ' (Ctrl+1)'" @click="activeTab = 'sources'"><svg class="tab-icon"><use href="/icons.svg#sources-icon"/></svg>{{ t('tab.sources') }}</button>
          <button :class="{ active: activeTab === 'releases' }" :title="t('tab.releases') + ' (Ctrl+2)'" @click="activeTab = 'releases'"><svg class="tab-icon"><use href="/icons.svg#release-icon"/></svg>{{ t('tab.releases') }}</button>
          <button :class="{ active: activeTab === 'logs' }" :title="t('tab.logs') + ' (Ctrl+3)'" @click="activeTab = 'logs'"><svg class="tab-icon"><use href="/icons.svg#log-icon"/></svg>{{ t('tab.logs') }}</button>
          <button :class="{ active: activeTab === 'settings' }" :title="t('tab.settings') + ' (Ctrl+4)'" @click="activeTab = 'settings'"><svg class="tab-icon"><use href="/icons.svg#settings-icon"/></svg>{{ t('tab.settings') }}</button>
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
        <ReleaseTab ref="releaseTab" v-model:search="releaseSearch" v-model:statusFilter="releaseStatusFilter" :releases="releases" :active="activeTab === 'releases'" :focus-target="focusTarget" :focus-token="focusReleaseToken" :loading="releasesLoading" :load-error="releasesError" @focus-consumed="handleFocusConsumed" @focus-not-found="handleFocusNotFound" @reload="loadReleases" @update="scheduleLoadReleases(); scheduleRefreshLogs()" />
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
