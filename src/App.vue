<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, provide } from 'vue'
import { ShowToastKey } from './injection-keys'
import { listen } from '@tauri-apps/api/event'
import {
  type Source,
  type ReleaseInfo,
  type LogEntry,
  type AppSettings,
  listSources,
  getReleases,
  getLogs,
  getSettings,
  triggerPoll,
  getPollCountdown,
} from './api'
import { t, setLocale } from './i18n'
import { isUnreadStatus } from './utils'
import SourceTab from './components/SourceTab.vue'
import ReleaseTab from './components/ReleaseTab.vue'
import LogTab from './components/LogTab.vue'
import SettingsTab from './components/SettingsTab.vue'

const activeTab = ref<'sources' | 'releases' | 'logs' | 'settings'>('sources')
const mainScrolled = ref(false)

function onMainScroll(e: Event) {
  const el = e.currentTarget as HTMLElement
  mainScrolled.value = el.scrollTop > 0
}

const sources = ref<Source[]>([])
const releases = ref<ReleaseInfo[]>([])
const logs = ref<LogEntry[]>([])
const settings = ref<AppSettings>({
  poll_interval_minutes: 30,
  proxy_url: '',
  minimize_to_tray: true,
  log_retention_days: 0,
  deepseek_enabled: false,
  deepseek_model: 'deepseek-v4-flash',
  deepseek_base_url: 'https://api.deepseek.com',
  deepseek_api_key_set: false,
  deepseek_proxy_enabled: false,
  check_prereleases: false,
  fetch_history: false,
  fetch_history_count: 1,
  language: 'zh-CN',
  theme: 'system',
  github_token_set: false,
})

const countdown = ref('')
const releaseSearch = ref('')
const polling = ref(false)
const sourceChecking = ref(false)
let countdownTimer: ReturnType<typeof setInterval> | null = null
let countdownSeconds = 0
let countdownReady = false

const unlisteners: (() => void)[] = []

const toastMessage = ref('')
const toastVisible = ref(false)
let toastTimer: ReturnType<typeof setTimeout> | null = null

const selectionMenu = ref<{ x: number; y: number } | null>(null)
function closeSelectionMenu() { selectionMenu.value = null }
async function handleCopySelection() {
  const text = window.getSelection()?.toString().trim()
  if (text) { try { await navigator.clipboard.writeText(text) } catch { /* ignore */ } }
  closeSelectionMenu()
}

function showToast(msg: string) {
  toastMessage.value = msg
  toastVisible.value = true
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toastVisible.value = false
  }, 3000)
}

provide(ShowToastKey, showToast)

function repoKey(owner: string, repo: string): string {
  return `${owner}/${repo}`.toLowerCase()
}

const unreadReleaseCounts = computed<Record<string, number>>(() => {
  const counts: Record<string, number> = {}
  for (const release of releases.value) {
    if (!isUnreadStatus(release.notification_status)) continue
    const key = repoKey(release.owner, release.repo)
    counts[key] = (counts[key] || 0) + 1
  }
  return counts
})

const totalReleaseCounts = computed<Record<string, number>>(() => {
  const counts: Record<string, number> = {}
  for (const release of releases.value) {
    const key = repoKey(release.owner, release.repo)
    counts[key] = (counts[key] || 0) + 1
  }
  return counts
})

function formatCountdown(secs: number) {
  if (secs <= 0) return t('app.check_soon')
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return t('app.min_sec', String(m), String(s))
}

async function loadAll() {
  await Promise.allSettled([loadSources(), loadReleases(), loadLogs(), loadSettings()])
}

async function loadSources() {
  sources.value = await listSources()
}
async function loadReleases() {
  releases.value = await getReleases()
}
async function loadLogs() {
  logs.value = await getLogs(100)
}
async function loadSettings() {
  settings.value = await getSettings()
  setLocale(settings.value.language)
  applyTheme(settings.value.theme)
}

function applyTheme(theme: string) {
  if (theme === 'dark') {
    document.documentElement.dataset.theme = 'dark'
  } else if (theme === 'light') {
    document.documentElement.dataset.theme = 'light'
  } else {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    document.documentElement.dataset.theme = prefersDark ? 'dark' : 'light'
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
    await loadLogs()
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
      showToast(t('app.new_found', [result.new_releases.length]))
    }
  } finally {
    polling.value = false
  }
}

function handleSourceCheckResult(count: number) {
  if (count === 0) {
    showToast(t('app.already_latest'))
  } else {
    showToast(t('app.new_found', [count]))
  }
}

function openSourceReleases(query: string) {
  releaseSearch.value = query
  activeTab.value = 'releases'
}

onMounted(async () => {
  await loadAll()
  watchSystemTheme()
  startCountdown()

  const handleContextMenu = (e: MouseEvent) => {
    const selection = window.getSelection()
    const selected = selection && selection.toString().trim()
    if (selected) {
      e.preventDefault()
      selectionMenu.value = { x: e.clientX, y: e.clientY }
      return
    }
    e.preventDefault()
  }
  document.addEventListener('contextmenu', handleContextMenu)
  unlisteners.push(() => document.removeEventListener('contextmenu', handleContextMenu))

  document.addEventListener('click', closeSelectionMenu)
  unlisteners.push(() => document.removeEventListener('click', closeSelectionMenu))

  const navigateUnlisten = await listen<string>('navigate', (event) => {
    if (event.payload === 'sources' || event.payload === 'releases' || event.payload === 'settings') {
      activeTab.value = event.payload as 'sources' | 'releases' | 'logs' | 'settings'
    }
  })
  unlisteners.push(navigateUnlisten)

  const pollUnlisten = await listen('poll-completed', () => {
    loadSources()
    loadReleases()
    loadLogs()
    syncCountdown(false)
  })
  unlisteners.push(pollUnlisten)

  const stateUnlisten = await listen('release-state-changed', () => {
    loadReleases()
    loadLogs()
  })
  unlisteners.push(stateUnlisten)
})

onUnmounted(() => {
  for (const unlisten of unlisteners) {
    unlisten()
  }
  unlisteners.length = 0
})
</script>

<template>
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
      <SourceTab v-show="activeTab === 'sources'" :sources="sources" :polling="polling || sourceChecking" :unread-release-counts="unreadReleaseCounts" :total-release-counts="totalReleaseCounts"
        @update="loadSources(); loadReleases(); loadLogs()"
        @check-result="handleSourceCheckResult"
        @check-busy="sourceChecking = $event"
        @open-releases="openSourceReleases" />
      <ReleaseTab v-show="activeTab === 'releases'" v-model:search="releaseSearch" :releases="releases" @update="loadReleases(); loadLogs()" />
      <LogTab v-show="activeTab === 'logs'" :logs="logs" @update="loadLogs()" />
      <SettingsTab v-show="activeTab === 'settings'" :settings="settings" @update="(pollChanged, forceReload) => { loadSettings(); if (pollChanged) startCountdown(); if (forceReload) { loadSources(); loadReleases(); } loadLogs(); applyTheme(settings.value.theme) }" />
    </main>

    <Transition name="toast">
      <div v-if="toastVisible" class="toast">{{ toastMessage }}</div>
    </Transition>

    <div v-if="selectionMenu" class="context-menu" :style="{ left: selectionMenu.x + 'px', top: selectionMenu.y + 'px' }" @click.stop>
      <button @click="handleCopySelection">{{ t('context.copy') }}</button>
    </div>
  </div>
</template>

<style scoped>
</style>
