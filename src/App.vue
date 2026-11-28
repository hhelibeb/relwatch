<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, watch, shallowRef, nextTick, type Component, type Ref } from 'vue'
import { storeToRefs } from 'pinia'
import ContextMenu, { type ContextMenuItem } from './components/common/ContextMenu.vue'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { syncSourceCapabilities } from './api/sources'
import { copyTextToClipboard } from './api/client'
import { mountEventBridge } from './api/event-bridge'
import { t } from './i18n'
import { useTransientUiGuard } from './composables/useTransientUiGuard'
import { useEscapeToTray } from './composables/useEscapeToTray'
import { useExternalLinkGuard } from './composables/useExternalLinkGuard'
import { applyTheme } from './composables/useTheme'
import { flushUsageTrackingNow } from './composables/useUsageTracking'
import { AGENT_DIVIDER_WIDTH, useAgentPanelLayout } from './composables/useAgentPanelLayout'
import { isEditableTarget } from './utils/dom'
import { extractErrorMessage } from './services/feedback'
import { useUiStore, type AppTab } from './stores/ui'
import { useCatalogStore } from './stores/catalog'
import { useSettingsStore } from './stores/settings'
import { useReleaseFilterStore } from './stores/releaseFilter'
import { useAgentStore } from './stores/agent'
import SourceTab from './components/SourceTab.vue'
import ReleaseTab from './components/ReleaseTab.vue'
import LogTab from './components/LogTab.vue'
import SettingsTab from './components/SettingsTab.vue'
import AgentWorkspace from './components/AgentWorkspace.vue'

const ui = useUiStore()
const catalog = useCatalogStore()
const settingsStore = useSettingsStore()
const releaseFilter = useReleaseFilterStore()
const agentStore = useAgentStore()

const {
  activeTab,
  logRefreshKey,
  toastMessage,
  toastVisible,
  toastStyle,
  countdown,
} = storeToRefs(ui)
const {
  sources,
  releases,
  sourcesLoading,
  sourcesError,
  releasesLoading,
  releasesError,
  agentConfig,
  polling,
  sourceChecking,
  agentEnabled,
  unreadReleaseCounts,
  totalReleaseCounts,
} = storeToRefs(catalog)
const { settings, minimizeToTray } = storeToRefs(settingsStore)
const {
  search: releaseSearch,
  statusFilter: releaseStatusFilter,
  focusTarget,
  focusToken: focusReleaseToken,
} = storeToRefs(releaseFilter)

const mainScrolled = ref(false)
function onMainScroll(e: Event) {
  const el = e.currentTarget as HTMLElement
  mainScrolled.value = el.scrollTop > 0
}

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
  onError: (msg) => ui.showToast(msg),
})

watch(
  [agentPanelVisible, agentPanelWidth],
  ([visible, width]) => {
    ui.setAgentPanelOffset(visible, width, AGENT_DIVIDER_WIDTH)
  },
  { immediate: true },
)

watch(
  agentPanelOpen,
  (open) => {
    agentStore.panelOpen = open
  },
  { immediate: true },
)

agentStore.bindPanelHandlers({
  open: (seed) => {
    void openAgentWorkspace(seed)
  },
  toggle: () => {
    void toggleAgentWorkspace()
  },
})

const releaseTab = ref<InstanceType<typeof ReleaseTab> | null>(null)
const unlisteners: (() => void)[] = []

const selectionMenu = ref<{ x: number; y: number } | null>(null)
const inputContextMenu = ref<{ x: number; y: number; target: HTMLElement } | null>(null)

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

const TAB_KEYS: Record<string, AppTab> = {
  '1': 'sources',
  '2': 'releases',
  '3': 'logs',
  '4': 'settings',
}

function onGlobalKeydown(e: KeyboardEvent) {
  if (e.isComposing) return
  if (import.meta.env.DEV && e.ctrlKey && e.shiftKey && (e.key === 'U' || e.key === 'u')) {
    e.preventDefault()
    void toggleStatsDev()
    return
  }
  if (!e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return
  const key = e.key.toLowerCase()
  if (key === 'f') {
    e.preventDefault()
    if (ui.dispatchFind()) return
    activeTab.value = 'releases'
    void nextTick(() => releaseTab.value?.focusSearch())
  } else if (key === 'r') {
    e.preventDefault()
    void catalog.handlePoll()
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
      ui.showToast(t('release.copy_failed') + extractErrorMessage(e))
    }
  }
  closeAllMenus()
}

function handleSelectionMenuAction(actionId: string) {
  if (actionId === 'copySelection') {
    void handleCopySelection()
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

async function loadAll() {
  await Promise.allSettled([
    catalog.loadSources(),
    catalog.loadReleases(),
    settingsStore.loadSettings(),
    catalog.loadAgentConfig(),
  ])
}

function handleSourceCheckResult(count: number) {
  if (count === 0) {
    ui.showToast(t('app.already_latest'))
  } else {
    ui.showToast(t('app.new_found', String(count)))
  }
}

useEscapeToTray(minimizeToTray)
useTransientUiGuard({
  onInvalidate: ui.invalidateToastHover,
})
useExternalLinkGuard()

let appDisposed = false

onMounted(async () => {
  window.addEventListener('keydown', onGlobalKeydown)
  unlisteners.push(() => window.removeEventListener('keydown', onGlobalKeydown))

  const unmountEvents = await mountEventBridge()
  if (appDisposed) {
    unmountEvents()
    return
  }
  unlisteners.push(unmountEvents)

  syncSourceCapabilities()
  await loadAll()
  if (appDisposed) return
  settingsStore.watchSystemTheme()
  ui.startCountdown()

  const handleContextMenu = (e: MouseEvent) => {
    const target = e.target as HTMLElement
    if (isEditableTarget(target)) {
      e.preventDefault()
      ui.closeAllContextMenus()
      inputContextMenu.value = { x: e.clientX, y: e.clientY, target }
      return
    }
    const selection = window.getSelection()
    const selected = selection && selection.toString().trim()
    if (selected) {
      e.preventDefault()
      ui.closeAllContextMenus()
      selectionMenu.value = { x: e.clientX, y: e.clientY }
      return
    }
    e.preventDefault()
  }
  document.addEventListener('contextmenu', handleContextMenu)
  unlisteners.push(() => document.removeEventListener('contextmenu', handleContextMenu))

  ui.registerCloser(closeAllMenus)
  document.addEventListener('click', closeAllMenus)
  unlisteners.push(() => {
    ui.unregisterCloser(closeAllMenus)
    document.removeEventListener('click', closeAllMenus)
  })
})

onUnmounted(() => {
  appDisposed = true
  for (const unlisten of unlisteners) {
    unlisten()
  }
  unlisteners.length = 0
  agentStore.bindPanelHandlers({ open: () => {}, toggle: () => {} })
  catalog.cancelScheduled()
  ui.disposeTimers()
  settingsStore.stopWatchSystemTheme()
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
                <svg class="release-agent-btn-icon"><use href="/icons.svg#agent-icon" /></svg>
                <svg class="release-agent-btn-arrow"><use :href="agentPanelOpen ? '/icons.svg#chevron-left-icon' : '/icons.svg#chevron-right-icon'" /></svg>
              </button>
              <span v-if="agentConfig?.enabled" class="header-actions-divider"></span>
              <button class="btn-primary" :disabled="polling || sourceChecking" :title="t('app.check_now') + ' (Ctrl+R)'" @click="catalog.handlePoll()">
                {{ polling || sourceChecking ? t('app.checking') : t('app.check_now') }}
              </button>
            </div>
          </div>
          <div class="header-bottom">
            <nav class="tabs">
              <button :class="{ active: activeTab === 'sources' }" :title="t('tab.sources') + ' (Ctrl+1)'" @click="activeTab = 'sources'"><svg class="tab-icon"><use href="/icons.svg#sources-icon" /></svg>{{ t('tab.sources') }}</button>
              <button :class="{ active: activeTab === 'releases' }" :title="t('tab.releases') + ' (Ctrl+2)'" @click="activeTab = 'releases'"><svg class="tab-icon"><use href="/icons.svg#release-icon" /></svg>{{ t('tab.releases') }}</button>
              <button :class="{ active: activeTab === 'logs' }" :title="t('tab.logs') + ' (Ctrl+3)'" @click="activeTab = 'logs'"><svg class="tab-icon"><use href="/icons.svg#log-icon" /></svg>{{ t('tab.logs') }}</button>
              <button :class="{ active: activeTab === 'settings' }" :title="t('tab.settings') + ' (Ctrl+4)'" @click="activeTab = 'settings'"><svg class="tab-icon"><use href="/icons.svg#settings-icon" /></svg>{{ t('tab.settings') }}</button>
            </nav>
            <span class="countdown-text" v-if="countdown">{{ t('app.next_check') }}{{ countdown }}</span>
          </div>
        </header>

        <main class="app-main" :class="{ 'is-scrolled': mainScrolled }" @scroll.passive="onMainScroll">
          <div v-show="activeTab === 'sources'">
            <SourceTab
              :sources="sources"
              :polling="polling || sourceChecking"
              :unread-release-counts="unreadReleaseCounts"
              :total-release-counts="totalReleaseCounts"
              :show-source-type-icons="settings.show_source_type_icons"
              :loading="sourcesLoading"
              :load-error="sourcesError"
              @update="catalog.loadSources(); catalog.scheduleLoadReleases(); ui.scheduleRefreshLogs()"
              @check-result="handleSourceCheckResult"
              @check-busy="sourceChecking = $event"
              @open-releases="releaseFilter.openSourceReleases"
              @open-unread-releases="releaseFilter.openSourceUnreadReleases"
            />
          </div>
          <div v-show="activeTab === 'releases'">
            <ReleaseTab
              ref="releaseTab"
              v-model:search="releaseSearch"
              v-model:statusFilter="releaseStatusFilter"
              :releases="releases"
              :active="activeTab === 'releases'"
              :focus-target="focusTarget"
              :focus-token="focusReleaseToken"
              :loading="releasesLoading"
              :load-error="releasesError"
              @focus-consumed="releaseFilter.consumeFocus"
              @focus-not-found="releaseFilter.reportFocusNotFound"
              @reload="catalog.loadReleases"
              @update="catalog.scheduleLoadReleases(); ui.scheduleRefreshLogs()"
            />
          </div>
          <div v-show="activeTab === 'logs'">
            <LogTab :refresh-key="logRefreshKey" @update="ui.refreshLogs()" />
          </div>
          <div v-show="activeTab === 'settings'">
            <SettingsTab
              :settings="settings"
              @update="(pollChanged, forceReload) => { if (pollChanged) ui.startCountdown(); if (forceReload) { settingsStore.loadSettings(); catalog.loadSources(); catalog.loadReleases(); } ui.refreshLogs(); applyTheme(settings.theme) }"
              @agent-config-changed="catalog.loadAgentConfig()"
              @update-log-written="ui.scheduleRefreshLogs()"
            />
          </div>
        </main>

        <Transition name="toast">
          <div
            v-if="toastVisible"
            class="toast"
            :style="toastStyle"
            role="status"
            aria-live="polite"
            @mouseenter="ui.handleToastMouseEnter"
            @mouseleave="ui.handleToastMouseLeave"
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
  overflow-wrap: anywhere;
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
