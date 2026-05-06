<script setup lang="ts">
import { ref, onMounted, provide } from 'vue'
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
import SourceTab from './components/SourceTab.vue'
import ReleaseTab from './components/ReleaseTab.vue'
import LogTab from './components/LogTab.vue'
import SettingsTab from './components/SettingsTab.vue'

const activeTab = ref<'sources' | 'releases' | 'logs' | 'settings'>('sources')

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
  language: 'zh-CN',
  github_token_set: false,
})

const countdown = ref('')
const polling = ref(false)
let countdownTimer: ReturnType<typeof setInterval> | null = null
let countdownSeconds = 0
let countdownReady = false

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

provide('showToast', showToast)

function formatCountdown(secs: number) {
  if (secs <= 0) return t('app.check_soon')
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return t('app.min_sec', String(m), String(s))
}

async function loadAll() {
  await Promise.all([loadSources(), loadReleases(), loadLogs(), loadSettings()])
}

async function loadSources() { sources.value = await listSources() }
async function loadReleases() { releases.value = await getReleases() }
async function loadLogs() { logs.value = await getLogs(100) }
async function loadSettings() {
  settings.value = await getSettings()
  setLocale(settings.value.language)
}

async function syncCountdown() {
  const secs = await getPollCountdown()
  const prev = countdownSeconds
  countdownSeconds = secs
  countdown.value = formatCountdown(secs)
  if (countdownReady && secs > prev + 30) {
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
  const enabled = sources.value.filter(s => s.enabled)
  if (enabled.length === 0) {
    showToast(t('app.no_sources'))
    return
  }
  polling.value = true
  try {
    const result = await triggerPoll()
    await loadReleases()
    await loadLogs()
    startCountdown()
    if (result.new_releases.length === 0) {
      showToast(t('app.already_latest'))
    }
  } finally {
    polling.value = false
  }
}

function handleSourceCheckResult(count: number) {
  if (count === 0) {
    showToast(t('app.already_latest'))
  }
}

onMounted(async () => {
  await loadAll()
  startCountdown()

  document.addEventListener('contextmenu', (e) => {
    const selection = window.getSelection()
    const selected = selection && selection.toString().trim()
    if (selected) {
      e.preventDefault()
      selectionMenu.value = { x: e.clientX, y: e.clientY }
      return
    }
    e.preventDefault()
  })

  document.addEventListener('click', closeSelectionMenu)

  listen<string>('navigate', (event) => {
    if (event.payload === 'sources' || event.payload === 'settings') {
      activeTab.value = event.payload as 'sources' | 'releases' | 'logs' | 'settings'
    }
  })

  listen('poll-completed', () => {
    loadReleases()
    loadLogs()
    syncCountdown()
  })
})
</script>

<template>
  <div class="app">
    <header class="app-header">
      <h1>{{ t('app.title') }}</h1>
      <nav class="tabs">
        <button :class="{ active: activeTab === 'sources' }" @click="activeTab = 'sources'"><svg class="tab-icon"><use href="/icons.svg#sources-icon"/></svg>{{ t('tab.sources') }}</button>
        <button :class="{ active: activeTab === 'releases' }" @click="activeTab = 'releases'"><svg class="tab-icon"><use href="/icons.svg#release-icon"/></svg>{{ t('tab.releases') }}</button>
        <button :class="{ active: activeTab === 'logs' }" @click="activeTab = 'logs'"><svg class="tab-icon"><use href="/icons.svg#log-icon"/></svg>{{ t('tab.logs') }}</button>
        <button :class="{ active: activeTab === 'settings' }" @click="activeTab = 'settings'"><svg class="tab-icon"><use href="/icons.svg#settings-icon"/></svg>{{ t('tab.settings') }}</button>
      </nav>
      <span class="countdown-text" v-if="countdown">{{ t('app.next_check') }}{{ countdown }}</span>
      <button class="btn-primary" :disabled="polling" @click="handlePoll">
        {{ polling ? t('app.checking') : t('app.check_now') }}
      </button>
    </header>

    <main class="app-main">
      <SourceTab v-show="activeTab === 'sources'" :sources="sources" :polling="polling"
        @update="loadSources(); loadReleases(); loadLogs()"
        @check-result="handleSourceCheckResult" />
      <ReleaseTab v-show="activeTab === 'releases'" :releases="releases" />
      <LogTab v-show="activeTab === 'logs'" :logs="logs" @update="loadLogs()" />
      <SettingsTab v-show="activeTab === 'settings'" :settings="settings" @update="(pollChanged) => { loadSettings(); if (pollChanged) startCountdown(); loadLogs() }" />
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
