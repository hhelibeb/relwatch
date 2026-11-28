<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { version } from '../../package.json'
import {
  type AppSettings,
  testDeepseekConnection,
  exportBackup,
  importBackup,
} from '../api/settings'
import { openReleaseUrl } from '../api/client'
import {
  AGENT_MAX_CONCURRENCY_LIMIT,
  AGENT_TIMEOUT_MAX,
  AGENT_TIMEOUT_MIN,
  getAgentConfig,
  saveAgentConfig,
  type AgentConfig,
} from '../api/agent'
import { t, setLocale } from '../i18n'
import { track } from '../composables/useUsageTracking'
import { applyTheme } from '../composables/useTheme'
import { applyFontScale } from '../composables/useFontScale'
import { useBilibiliLogin } from '../composables/useBilibiliLogin'
import { useAppUpdate } from '../composables/useAppUpdate'
import { confirmAction, extractErrorMessage } from '../services/feedback'
import { useUiStore } from '../stores/ui'
import { useCatalogStore } from '../stores/catalog'
import { useSettingsStore } from '../stores/settings'
import GeneralPanel from './settings/GeneralPanel.vue'
import AccountsPanel from './settings/AccountsPanel.vue'
import DataPanel from './settings/DataPanel.vue'
import AppearancePanel from './settings/AppearancePanel.vue'
import AiPanel from './settings/AiPanel.vue'
import AgentPanel, { type AgentDraftForm } from './settings/AgentPanel.vue'
import AboutPanel from './settings/AboutPanel.vue'
import AiUsageStatsModal from './AiUsageStatsModal.vue'

const props = defineProps<{ settings: AppSettings }>()
const emit = defineEmits<{
  update: [pollIntervalChanged: boolean, forceReload?: boolean]
  agentConfigChanged: []
  updateLogWritten: []
}>()

const ui = useUiStore()
const catalog = useCatalogStore()
const settingsStore = useSettingsStore()
const { draft: form } = storeToRefs(settingsStore)

const settingsTab = ref<'general' | 'accounts' | 'data' | 'appearance' | 'ai' | 'agent' | 'about'>('general')
const savingSettings = ref(false)
const deepseekApiKey = ref('')
const githubToken = ref('')
const youtubeApiKey = ref('')
const bilibiliCookie = ref('')
const testingDeepseek = ref(false)
const showAiUsage = ref(false)
const prevPollInterval = ref(props.settings.poll_interval_minutes)
const appearancePanelRef = ref<InstanceType<typeof AppearancePanel> | null>(null)

let lastSyncedSettings: AppSettings | null = null

function isSameSettings(a: AppSettings, b: AppSettings): boolean {
  const keys = Object.keys(a) as (keyof AppSettings)[]
  if (keys.length !== Object.keys(b).length) return false
  return keys.every((key) => a[key] === b[key])
}

watch(
  () => props.settings,
  (s) => {
    const prev = lastSyncedSettings
    if (prev && isSameSettings(prev, s) && isSameSettings(settingsStore.settings, s)) {
      return
    }
    // 仅在值不一致时同步 store，避免 App.vue 透传 settingsStore.settings 时回写新对象触发无限重渲染
    if (!isSameSettings(settingsStore.settings, s)) {
      settingsStore.settings = { ...s }
    }
    if (!prev) {
      settingsStore.syncDraftFromSettings(s)
    } else {
      const nextDraft = { ...form.value }
      const keys = Object.keys(s) as (keyof AppSettings)[]
      for (const key of keys) {
        if (nextDraft[key] === prev[key]) {
          ;(nextDraft as Record<string, unknown>)[key] = s[key]
        }
      }
      form.value = nextDraft
    }
    lastSyncedSettings = { ...s }
    prevPollInterval.value = s.poll_interval_minutes
  },
  { immediate: true, deep: true },
)

const agentDraft = ref<AgentDraftForm>({
  enabled: false,
  agentType: 'pi',
  binary: '',
  model: '',
  workingDir: '',
  promptSuffix: '',
  timeout: 300,
  maxConcurrency: 3,
  skills: [],
})

function normalizedAgentTimeout(): number {
  const v = agentDraft.value.timeout
  if (!Number.isFinite(v)) return AGENT_TIMEOUT_MIN
  return Math.min(AGENT_TIMEOUT_MAX, Math.max(AGENT_TIMEOUT_MIN, Math.floor(v)))
}

function normalizedAgentMaxConcurrency(): number {
  const v = agentDraft.value.maxConcurrency
  if (!Number.isFinite(v)) return 1
  return Math.min(AGENT_MAX_CONCURRENCY_LIMIT, Math.max(1, Math.floor(v)))
}

function agentSnapshot(): string {
  const d = agentDraft.value
  return JSON.stringify({
    enabled: d.enabled,
    type: d.agentType.trim() || 'pi',
    binary: d.binary.trim() || null,
    model: d.model.trim() || null,
    wd: d.workingDir.trim() || null,
    suffix: d.promptSuffix.trim() || null,
    timeout: normalizedAgentTimeout(),
    maxConcurrency: normalizedAgentMaxConcurrency(),
    skills: d.skills,
  })
}

const agentSavedSnapshot = ref(agentSnapshot())
const agentDirty = computed(() => agentSnapshot() !== agentSavedSnapshot.value)

function applyAgentConfig(cfg: AgentConfig) {
  agentDraft.value = {
    enabled: cfg.enabled,
    agentType: cfg.agent_type,
    binary: cfg.binary ?? '',
    model: cfg.model ?? '',
    workingDir: cfg.working_dir ?? '',
    promptSuffix: cfg.prompt_suffix ?? '',
    timeout: cfg.timeout_seconds,
    maxConcurrency: cfg.max_concurrency,
    skills: [...cfg.skills],
  }
  agentSavedSnapshot.value = agentSnapshot()
}

async function loadAgentConfig() {
  if (catalog.agentConfig) {
    applyAgentConfig(catalog.agentConfig)
    return
  }
  try {
    applyAgentConfig(await getAgentConfig())
  } catch {
    // 加载失败保持默认空表单
  }
}

void loadAgentConfig()

const { biliLoginBusy, handleBilibiliLogin, handleClearBilibiliCookie } = useBilibiliLogin({
  showToast: (msg) => ui.showToast(msg),
  onLoginSuccess: () => {
    form.value.bilibili_cookie_set = true
    settingsStore.settings.bilibili_cookie_set = true
    bilibiliCookie.value = ''
  },
  onCookieCleared: () => {
    form.value.bilibili_cookie_set = false
    settingsStore.settings.bilibili_cookie_set = false
    bilibiliCookie.value = ''
  },
})

const appUpdate = useAppUpdate(
  () => ({ mode: settingsStore.settings.proxy_mode, url: settingsStore.settings.proxy_url.trim() }),
  () => emit('updateLogWritten'),
)

const TAB_SETTING_KEYS: Record<'general' | 'accounts' | 'appearance' | 'ai', readonly string[]> = {
  general: ['auto_start', 'poll_interval_minutes', 'proxy_mode', 'proxy_url', 'log_retention_days', 'check_prereleases', 'fetch_history', 'fetch_history_count', 'enable_usage_stats', 'auto_read_detail'],
  accounts: ['github_token', 'youtube_api_key', 'bilibili_cookie'],
  appearance: ['language', 'theme', 'font_scale', 'minimize_to_tray', 'show_source_type_icons'],
  ai: ['deepseek_enabled', 'deepseek_api_key', 'deepseek_model', 'deepseek_base_url', 'deepseek_proxy_bypass', 'deepseek_prompt', 'deepseek_min_importance', 'show_importance', 'deepseek_translate_release'],
}

const dirtyFields = computed(() => {
  const dirty = new Set<string>()
  const f = form.value as unknown as Record<string, unknown>
  const base = settingsStore.settings as unknown as Record<string, unknown>
  for (const key of Object.keys(f)) {
    if (f[key] !== base[key]) dirty.add(key)
  }
  if (deepseekApiKey.value) dirty.add('deepseek_api_key')
  if (githubToken.value) dirty.add('github_token')
  if (youtubeApiKey.value) dirty.add('youtube_api_key')
  if (bilibiliCookie.value) dirty.add('bilibili_cookie')
  return dirty
})

const dirtyCount = computed(() => dirtyFields.value.size + (agentDirty.value ? 1 : 0))

const dirtyByTab = computed(() => {
  const f = dirtyFields.value
  return {
    general: TAB_SETTING_KEYS.general.filter((k) => f.has(k)).length,
    accounts: TAB_SETTING_KEYS.accounts.filter((k) => f.has(k)).length,
    appearance: TAB_SETTING_KEYS.appearance.filter((k) => f.has(k)).length,
    ai: TAB_SETTING_KEYS.ai.filter((k) => f.has(k)).length,
    agent: agentDirty.value ? 1 : 0,
    about: 0,
  }
})

async function handleSave() {
  savingSettings.value = true
  track('settings.save')
  const prevSettings = { ...settingsStore.settings }
  let mainSettingsSaved = false
  let pollChanged = false
  try {
    if (form.value.deepseek_prompt && !form.value.deepseek_prompt.includes('{}')) {
      ui.showToast(t('settings.deepseek_prompt_validate_failed'))
      savingSettings.value = false
      return
    }
    const payload: AppSettings = {
      ...form.value,
      proxy_url: form.value.proxy_url.trim(),
      deepseek_model: form.value.deepseek_model.trim() || 'deepseek-v4-flash',
      deepseek_base_url: form.value.deepseek_base_url.trim() || 'https://api.deepseek.com',
      fetch_history_count: form.value.fetch_history_count ?? 1,
    }
    const pollChangedFromStore = await settingsStore.saveSettings(payload)
    mainSettingsSaved = true
    pollChanged = pollChangedFromStore || payload.poll_interval_minutes !== prevPollInterval.value
    if (pollChanged) prevPollInterval.value = payload.poll_interval_minutes

    const credentials: Array<{
      kind: 'deepseek_api_key' | 'github_token' | 'youtube_api_key' | 'bilibili_cookie'
      refVal: typeof deepseekApiKey
    }> = [
      { kind: 'deepseek_api_key', refVal: deepseekApiKey },
      { kind: 'github_token', refVal: githubToken },
      { kind: 'youtube_api_key', refVal: youtubeApiKey },
      { kind: 'bilibili_cookie', refVal: bilibiliCookie },
    ]
    for (const c of credentials) {
      if (c.refVal.value) {
        await settingsStore.saveCredential(c.kind, c.refVal.value)
        c.refVal.value = ''
      }
    }

    if (agentDirty.value) {
      const d = agentDraft.value
      await saveAgentConfig({
        enabled: d.enabled,
        agent_type: d.agentType.trim() || 'pi',
        binary: d.binary.trim() || null,
        model: d.model.trim() || null,
        working_dir: d.workingDir.trim() || null,
        prompt_suffix: d.promptSuffix.trim() || null,
        timeout_seconds: normalizedAgentTimeout(),
        max_concurrency: normalizedAgentMaxConcurrency(),
        skills: [...d.skills],
      })
      await catalog.loadAgentConfig()
      await loadAgentConfig()
      emit('agentConfigChanged')
    }

    ui.showToast(t('settings.saved'))
    emit('update', pollChanged)
  } catch (e: unknown) {
    if (mainSettingsSaved) {
      emit('update', pollChanged)
    } else {
      setLocale(prevSettings.language)
      applyTheme(prevSettings.theme)
      applyFontScale(prevSettings.font_scale)
    }
    ui.showToast(t('settings.save_failed') + extractErrorMessage(e))
  } finally {
    savingSettings.value = false
  }
}

function discardChanges() {
  track('settings.discard')
  const langDirty = dirtyFields.value.has('language')
  const themeDirty = dirtyFields.value.has('theme')
  const fontScaleDirty = dirtyFields.value.has('font_scale')
  settingsStore.syncDraftFromSettings()
  if (langDirty) setLocale(form.value.language)
  if (themeDirty) {
    appearancePanelRef.value?.clearThemePreview()
    applyTheme(form.value.theme)
  }
  if (fontScaleDirty) applyFontScale(form.value.font_scale)
  deepseekApiKey.value = ''
  githubToken.value = ''
  youtubeApiKey.value = ''
  bilibiliCookie.value = ''
  void loadAgentConfig()
}

async function handleTestDeepseek() {
  testingDeepseek.value = true
  track('settings.test_ai')
  try {
    await testDeepseekConnection({
      model: form.value.deepseek_model.trim(),
      baseUrl: form.value.deepseek_base_url.trim(),
      apiKey: deepseekApiKey.value,
      proxyBypass: form.value.deepseek_proxy_bypass,
      proxyUrl: form.value.proxy_url.trim(),
      proxyMode: form.value.proxy_mode,
    })
    ui.showToast(t('settings.connection_success'))
  } catch (e: unknown) {
    ui.showToast(t('settings.connect_failed') + extractErrorMessage(e))
  } finally {
    testingDeepseek.value = false
  }
}

async function handleExportBackup() {
  track('settings.export')
  try {
    const path = await exportBackup()
    ui.showToast(t('backup.export_success') + ': ' + path)
    emit('update', false)
  } catch (e: unknown) {
    const msg = extractErrorMessage(e)
    if (msg === t('err.backup_cancelled_export')) {
      ui.showToast(t('backup.export_cancelled'))
    } else {
      ui.showToast(t('backup.export_failed') + msg)
    }
  }
}

async function handleImportBackup() {
  const confirmed = await confirmAction(t('backup.import_confirm'), {
    title: t('backup.import_confirm_title'),
    kind: 'warning',
  })
  if (!confirmed) return
  track('settings.import')
  try {
    await importBackup()
    ui.showToast(t('backup.import_success'))
    emit('update', false, true)
  } catch (e: unknown) {
    const msg = extractErrorMessage(e)
    if (msg === t('err.backup_cancelled_import')) {
      ui.showToast(t('backup.import_cancelled'))
    } else {
      ui.showToast(t('backup.import_failed') + msg)
    }
  }
}
</script>

<template>
  <section class="tab-content">
    <div class="settings-layout">
      <aside class="settings-sidebar">
        <button :class="{ active: settingsTab === 'general' }" @click="settingsTab = 'general'">{{ t('settings.general') }}<span v-if="dirtyByTab.general" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'accounts' }" @click="settingsTab = 'accounts'">{{ t('settings.accounts') }}<span v-if="dirtyByTab.accounts" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'appearance' }" @click="settingsTab = 'appearance'">{{ t('settings.appearance') }}<span v-if="dirtyByTab.appearance" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'ai' }" @click="settingsTab = 'ai'">{{ t('settings.ai') }}<span v-if="dirtyByTab.ai" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'agent' }" @click="settingsTab = 'agent'">{{ t('settings.agent') }}<span v-if="dirtyByTab.agent" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'data' }" @click="settingsTab = 'data'">{{ t('settings.data') }}</button>
        <button :class="{ active: settingsTab === 'about' }" data-testid="settings-tab-about" @click="settingsTab = 'about'">{{ t('settings.about') }}</button>
        <div class="version-row">
          <button class="version-github-btn" title="GitHub" @click="openReleaseUrl('https://github.com/hhelibeb/relwatch')">
            <svg viewBox="0 0 19 19" width="16" height="16" fill="currentColor">
              <path fill-rule="evenodd" d="M9.356 1.85C5.05 1.85 1.57 5.356 1.57 9.694a7.84 7.84 0 0 0 5.324 7.44c.387.079.528-.168.528-.376 0-.182-.013-.805-.013-1.454-2.165.467-2.616-.935-2.616-.935-.349-.91-.864-1.143-.864-1.143-.71-.48.051-.48.051-.48.787.051 1.2.805 1.2.805.695 1.194 1.817.857 2.268.649.064-.507.27-.857.49-1.052-1.728-.182-3.545-.857-3.545-3.87 0-.857.31-1.558.8-2.104-.078-.195-.349-1 .077-2.078 0 0 .657-.208 2.14.805a7.5 7.5 0 0 1 1.946-.26c.657 0 1.328.092 1.946.26 1.483-1.013 2.14-.805 2.14-.805.426 1.078.155 1.883.078 2.078.502.546.799 1.247.799 2.104 0 3.013-1.818 3.675-3.558 3.87.284.247.528.714.528 1.454 0 1.052-.012 1.896-.012 2.156 0 .208.142.455.528.377a7.84 7.84 0 0 0 5.324-7.441c.013-4.338-3.48-7.844-7.773-7.844"/>
            </svg>
          </button>
          <span class="version-text">v{{ version }}</span>
        </div>
      </aside>
      <div class="settings-main">
        <div v-if="dirtyCount > 0" class="settings-banner">
          <span class="settings-banner-text">{{ t('settings.unsaved_banner', String(dirtyCount)) }}</span>
          <div class="settings-banner-actions">
            <button class="btn-secondary" @click="discardChanges">{{ t('settings.discard') }}</button>
            <button class="btn-primary" @click="handleSave">{{ t('settings.save') }}</button>
          </div>
        </div>

        <GeneralPanel v-if="settingsTab === 'general'" v-model:form="form" :dirty-fields="dirtyFields" />
        <AccountsPanel
          v-else-if="settingsTab === 'accounts'"
          v-model:github-token="githubToken"
          v-model:youtube-api-key="youtubeApiKey"
          v-model:bilibili-cookie="bilibiliCookie"
          :form="form"
          :dirty-fields="dirtyFields"
          :bili-login-busy="biliLoginBusy"
          @bilibili-login="handleBilibiliLogin"
          @clear-bilibili-cookie="handleClearBilibiliCookie"
        />
        <DataPanel
          v-else-if="settingsTab === 'data'"
          :github-token-set="form.github_token_set"
          @export-backup="handleExportBackup"
          @import-backup="handleImportBackup"
        />
        <AppearancePanel
          v-else-if="settingsTab === 'appearance'"
          ref="appearancePanelRef"
          v-model:form="form"
          :dirty-fields="dirtyFields"
        />
        <AiPanel
          v-else-if="settingsTab === 'ai'"
          v-model:form="form"
          v-model:deepseek-api-key="deepseekApiKey"
          :dirty-fields="dirtyFields"
          :testing-deepseek="testingDeepseek"
          @open-ai-usage="showAiUsage = true"
          @test-connection="handleTestDeepseek"
        />
        <AgentPanel v-else-if="settingsTab === 'agent'" v-model:draft="agentDraft" />
        <AboutPanel v-else-if="settingsTab === 'about'" :update="appUpdate" />

        <div v-if="settingsTab !== 'data' && settingsTab !== 'about'" class="setting-actions">
          <button class="btn-primary" :disabled="savingSettings" @click="handleSave">
            {{ savingSettings ? t('settings.saving') : t('settings.save') }}
          </button>
        </div>
      </div>
    </div>

    <AiUsageStatsModal v-if="showAiUsage" @close="showAiUsage = false" />
  </section>
</template>

<style src="./settings/settings-shared.css"></style>
<style scoped>
.settings-layout {
  display: flex;
  gap: 16px;
  align-items: flex-start;
}

.settings-sidebar {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 120px;
  position: sticky;
  top: 0;
  align-self: flex-start;
  padding: 0 12px 0 0;
}

.settings-sidebar button {
  position: relative;
  padding: 7px 10px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-body);
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
  transition: background var(--dur-state) var(--ease-out), color var(--dur-state) var(--ease-out);
}

.sidebar-dirty-dot {
  position: absolute;
  right: 10px;
  top: 50%;
  transform: translateY(-50%);
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warning);
}

.settings-sidebar button:hover {
  background: var(--hover-surface);
  color: var(--text);
}

.settings-sidebar button.active {
  background: var(--bg-subtle);
  color: var(--text);
  font-weight: 600;
}

.version-row {
  margin-top: auto;
  padding: 8px 14px 4px 11px;
  border-top: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 6px;
}

.version-text {
  font-size: var(--fs-caption);
  color: var(--text-muted);
}

.version-github-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-xs);
  transition: color var(--dur-state) var(--ease-out), background var(--dur-state) var(--ease-out);
}

.version-github-btn:hover {
  color: var(--text);
  background: var(--hover-surface);
}

.version-github-btn svg {
  width: 18px;
  height: 18px;
}

.settings-sidebar .version-github-btn {
  padding: 0;
}

.settings-main {
  flex: 1;
  min-width: 0;
}

.settings-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 14px;
  margin-bottom: 12px;
  background: var(--warning-soft-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.settings-banner-text {
  font-size: var(--fs-body);
  color: var(--text);
  line-height: 1.4;
}

.settings-banner-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.settings-banner-actions .btn-primary,
.settings-banner-actions .btn-secondary {
  padding: 6px 14px;
  font-size: var(--fs-meta);
  white-space: nowrap;
}

.setting-actions {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-top: 16px;
}
</style>
