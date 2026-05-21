<script setup lang="ts">
import { ref, reactive, inject, watch, nextTick, onUnmounted, computed } from 'vue'
import { ShowToastKey } from '../injection-keys'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { version } from '../../package.json'
import {
  type AppSettings,
  updateSettings,
  setDeepseekApiKey,
  setGithubToken,
  testDeepseekConnection,
  exportBackup,
  importBackup,
  openReleaseUrl,
} from '../api'
import { t, setLocale, languages } from '../i18n'

const props = defineProps<{ settings: AppSettings }>()
const emit = defineEmits<{ update: [pollIntervalChanged: boolean, forceReload?: boolean] }>()
const showToast = inject(ShowToastKey)!

const settingsTab = ref<'general' | 'data' | 'appearance' | 'ai'>('general')
const savingSettings = ref(false)
const deepseekApiKey = ref('')
const githubToken = ref('')
const testingDeepseek = ref(false)
const prevPollInterval = ref(props.settings.poll_interval_minutes)

// 本地 form 副本用于 v-model 双向绑定，避免直接修改 props
const form = reactive({ ...props.settings })
watch(() => props.settings, (s) => {
  Object.assign(form, s)
}, { deep: true })

const themeDropdownOpen = ref(false)
const previewTheme = ref<string | null>(null)
const themeOptions = [
  { value: 'system', label: 'settings.theme_system' },
  { value: 'light', label: 'settings.theme_light' },
  { value: 'dark', label: 'settings.theme_dark' },
]

const themeSelectRef = ref<HTMLElement | null>(null)

let outsideClickHandler: ((e: MouseEvent) => void) | null = null

function handleOutsideClick(e: MouseEvent) {
  if (themeSelectRef.value && !themeSelectRef.value.contains(e.target as Node)) {
    themeDropdownOpen.value = false
    clearThemePreview()
  }
}

watch(themeDropdownOpen, (isOpen) => {
  if (isOpen) {
    nextTick(() => {
      outsideClickHandler = handleOutsideClick
      document.addEventListener('click', outsideClickHandler)
    })
  } else {
    if (outsideClickHandler) {
      document.removeEventListener('click', outsideClickHandler)
      outsideClickHandler = null
    }
  }
})

onUnmounted(() => {
  if (outsideClickHandler) {
    document.removeEventListener('click', outsideClickHandler)
    outsideClickHandler = null
  }
  if (langOutsideClickHandler) {
    document.removeEventListener('click', langOutsideClickHandler)
    langOutsideClickHandler = null
  }
})

function setThemePreview(val: string) {
  previewTheme.value = val
  if (val === 'dark') {
    document.documentElement.dataset.theme = 'dark'
  } else if (val === 'light') {
    document.documentElement.dataset.theme = 'light'
  } else {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    document.documentElement.dataset.theme = prefersDark ? 'dark' : 'light'
  }
}

// ── 语言选择（悬停预览）─────────────────────────────

const langDropdownOpen = ref(false)
const previewLang = ref<string | null>(null)
const langSelectRef = ref<HTMLElement | null>(null)

let langOutsideClickHandler: ((e: MouseEvent) => void) | null = null

function handleLangOutsideClick(e: MouseEvent) {
  if (langSelectRef.value && !langSelectRef.value.contains(e.target as Node)) {
    langDropdownOpen.value = false
    clearLangPreview()
  }
}

watch(langDropdownOpen, (isOpen) => {
  if (isOpen) {
    nextTick(() => {
      langOutsideClickHandler = handleLangOutsideClick
      document.addEventListener('click', langOutsideClickHandler)
    })
  } else {
    if (langOutsideClickHandler) {
      document.removeEventListener('click', langOutsideClickHandler)
      langOutsideClickHandler = null
    }
  }
})

function setLangPreview(val: string) {
  previewLang.value = val
  setLocale(val)
}

function clearLangPreview() {
  previewLang.value = null
  setLocale(form.language)
}

function selectLang(val: string) {
  form.language = val
  previewLang.value = null
  setLocale(val)
  setTimeout(() => {
    langDropdownOpen.value = false
  }, 0)
}

function toggleLangDropdown() {
  langDropdownOpen.value = !langDropdownOpen.value
  if (!langDropdownOpen.value) {
    clearLangPreview()
  }
}

// ── 主题选择（悬停预览）─────────────────────────────

function clearThemePreview() {
  previewTheme.value = null
  const theme = form.theme
  if (theme === 'dark') {
    document.documentElement.dataset.theme = 'dark'
  } else if (theme === 'light') {
    document.documentElement.dataset.theme = 'light'
  } else {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    document.documentElement.dataset.theme = prefersDark ? 'dark' : 'light'
  }
}

function selectTheme(val: string) {
  form.theme = val
  previewTheme.value = null
  setThemePreview(val)
  setTimeout(() => {
    themeDropdownOpen.value = false
  }, 0)
}

function toggleDropdown() {
  themeDropdownOpen.value = !themeDropdownOpen.value
  if (!themeDropdownOpen.value) {
    clearThemePreview()
  }
}

async function handleSave() {
  savingSettings.value = true
  try {
    const s = form
    // 先设置敏感凭据，最后持久化全部设置到 DB
    if (deepseekApiKey.value) {
      await setDeepseekApiKey(deepseekApiKey.value)
      deepseekApiKey.value = ''
      form.deepseek_api_key_set = true
    }
    if (githubToken.value) {
      await setGithubToken(githubToken.value)
      githubToken.value = ''
      form.github_token_set = true
    }
    setLocale(form.language)
    await updateSettings({
      pollIntervalMinutes: s.poll_interval_minutes,
      proxyUrl: s.proxy_url.trim(),
      minimizeToTray: s.minimize_to_tray,
      logRetentionDays: s.log_retention_days,
      deepseekEnabled: s.deepseek_enabled,
      deepseekModel: s.deepseek_model.trim() || 'deepseek-v4-flash',
      deepseekBaseUrl: s.deepseek_base_url.trim() || 'https://api.deepseek.com',
      deepseekProxyEnabled: s.deepseek_proxy_enabled,
      checkPrereleases: s.check_prereleases,
      fetchHistory: s.fetch_history,
      fetchHistoryCount: s.fetch_history_count ?? 1,
      language: s.language,
      theme: s.theme,
    })
    showToast(t('settings.saved'))
    const pollChanged = form.poll_interval_minutes !== prevPollInterval.value
    if (pollChanged) prevPollInterval.value = form.poll_interval_minutes
    emit('update', pollChanged)
  } catch (e: unknown) {
    showToast(t('settings.save_failed') + (e instanceof Error ? e.message : String(e)))
  } finally {
    savingSettings.value = false
  }
}

async function handleTestDeepseek() {
  testingDeepseek.value = true
  try {
    const msg = await testDeepseekConnection()
    await message(msg, { title: 'DeepSeek Connection Test', kind: 'info' })
  } catch (e: unknown) {
    await message(t('settings.connect_failed') + (e instanceof Error ? e.message : String(e)), { title: 'DeepSeek Connection Test', kind: 'error' })
  } finally {
    testingDeepseek.value = false
  }
}

async function handleExportBackup() {
  try {
    const path = await exportBackup()
    showToast(t('backup.export_success') + ': ' + path)
    emit('update', false)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e)
    if (msg.includes('取消')) {
      showToast(t('backup.export_cancelled'))
    } else {
      showToast(t('backup.export_failed') + msg)
    }
  }
}

async function handleImportBackup() {
  const confirmed = await confirm(t('backup.import_confirm'), { title: t('backup.import_confirm_title'), kind: 'warning' })
  if (!confirmed) return
  try {
    await importBackup()
    showToast(t('backup.import_success'))
    emit('update', false, true)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e)
    if (msg.includes('取消')) {
      showToast(t('backup.import_cancelled'))
    } else {
      showToast(t('backup.import_failed') + msg)
    }
  }
}
</script>

<template>
  <section class="tab-content">
    <div class="settings-layout">
      <aside class="settings-sidebar">
        <button :class="{ active: settingsTab === 'general' }" @click="settingsTab = 'general'">{{ t('settings.general') }}</button>
        <button :class="{ active: settingsTab === 'data' }" @click="settingsTab = 'data'">{{ t('settings.data') }}</button>
        <button :class="{ active: settingsTab === 'appearance' }" @click="settingsTab = 'appearance'">{{ t('settings.appearance') }}</button>
        <button :class="{ active: settingsTab === 'ai' }" @click="settingsTab = 'ai'">{{ t('settings.ai') }}</button>
        <div class="version-row">
          <button class="version-github-btn" @click="openReleaseUrl('https://github.com/hhelibeb/relwatch')" title="GitHub">
            <svg viewBox="0 0 19 19" width="16" height="16" fill="currentColor">
              <path fill-rule="evenodd" d="M9.356 1.85C5.05 1.85 1.57 5.356 1.57 9.694a7.84 7.84 0 0 0 5.324 7.44c.387.079.528-.168.528-.376 0-.182-.013-.805-.013-1.454-2.165.467-2.616-.935-2.616-.935-.349-.91-.864-1.143-.864-1.143-.71-.48.051-.48.051-.48.787.051 1.2.805 1.2.805.695 1.194 1.817.857 2.268.649.064-.507.27-.857.49-1.052-1.728-.182-3.545-.857-3.545-3.87 0-.857.31-1.558.8-2.104-.078-.195-.349-1 .077-2.078 0 0 .657-.208 2.14.805a7.5 7.5 0 0 1 1.946-.26c.657 0 1.328.092 1.946.26 1.483-1.013 2.14-.805 2.14-.805.426 1.078.155 1.883.078 2.078.502.546.799 1.247.799 2.104 0 3.013-1.818 3.675-3.558 3.87.284.247.528.714.528 1.454 0 1.052-.012 1.896-.012 2.156 0 .208.142.455.528.377a7.84 7.84 0 0 0 5.324-7.441c.013-4.338-3.48-7.844-7.773-7.844"/>
            </svg>
          </button>
          <span class="version-text">v{{ version }}</span>
        </div>
      </aside>
      <div class="settings-main">
        <div v-if="settingsTab === 'general'" class="settings-form">
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.poll_interval') }}</span>
            <input
              type="number"
              v-model.number="form.poll_interval_minutes"
              min="5"
              max="1440"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.proxy') }}</span>
            <input
              type="text"
              v-model="form.proxy_url"
              :placeholder="t('settings.proxy_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.github_token') }}</span>
            <input
              type="password"
              v-model="githubToken"
              :placeholder="form.github_token_set ? t('settings.github_token_set') : t('settings.github_token_input')"
              class="setting-input"
            />
            <span class="setting-note">{{ t('settings.github_token_note') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.log_retention') }}</span>
            <input
              type="number"
              v-model.number="form.log_retention_days"
              min="0"
              max="3650"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.check_prereleases" />
            <span class="setting-label">{{ t('settings.check_prereleases') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.fetch_history" />
            <span class="setting-label">{{ t('settings.fetch_history') }}</span>
          </label>
          <label class="setting-row" v-if="form.fetch_history">
            <span class="setting-label">{{ t('settings.fetch_history_count') }}</span>
            <input
              type="number"
              v-model.number="form.fetch_history_count"
              min="1"
              max="50"
              class="setting-input setting-input-narrow"
            />
          </label>
        </div>
        <div v-if="settingsTab === 'data'" class="settings-form" style="gap:13px">
          <h3 class="setting-section-title">{{ t('backup.section_title') }}</h3>
          <p class="setting-section-desc">
            {{ t('backup.section_desc') }}<template v-if="form.github_token_set"><br>{{ t('backup.token_note') }}</template>
          </p>
          <div class="setting-row backup-actions">
            <button class="btn-secondary" @click="handleExportBackup">{{ t('backup.export_btn') }}</button>
            <button class="btn-secondary btn-danger" @click="handleImportBackup">{{ t('backup.import_btn') }}</button>
          </div>
        </div>
        <div v-if="settingsTab === 'ai'" class="settings-form">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_enabled" />
            <span class="setting-label">{{ t('settings.enable_ai') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.api_key') }}</span>
            <input
              type="password"
              v-model="deepseekApiKey"
              :placeholder="form.deepseek_api_key_set ? t('settings.api_key_set') : t('settings.api_key_input')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.model') }}</span>
            <input
              type="text"
              v-model="form.deepseek_model"
              placeholder="deepseek-v4-flash"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.api_url') }}</span>
            <input
              type="text"
              v-model="form.deepseek_base_url"
              placeholder="https://api.deepseek.com"
              class="setting-input"
            />
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_proxy_enabled" />
            <span class="setting-label">{{ t('settings.use_proxy') }}</span>
          </label>
          <div class="setting-row">
            <button class="btn-secondary" :disabled="testingDeepseek" @click="handleTestDeepseek">
              {{ testingDeepseek ? t('settings.testing') : t('settings.test_connection') }}
            </button>
          </div>
        </div>
        <div v-if="settingsTab === 'appearance'" class="settings-form">
          <div class="setting-row">
            <span class="setting-label">{{ t('settings.language') }}<svg class="label-icon"><use href="/icons.svg#language-icon"/></svg></span>
            <div ref="langSelectRef" class="theme-select" @mouseleave="clearLangPreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleLangDropdown">
                <span>{{ previewLang ? languages.find(l => l.value === previewLang)?.label : languages.find(l => l.value === form.language)?.label }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="langDropdownOpen" class="theme-select-dropdown">
                <div
                  v-for="lang in languages"
                  :key="lang.value"
                  class="theme-select-option"
                  :class="{ selected: form.language === lang.value && !previewLang, previewed: previewLang === lang.value }"
                  @click.stop="selectLang(lang.value)"
                  @mouseenter="setLangPreview(lang.value)"
                >
                  {{ lang.label }}
                </div>
              </div>
            </div>
          </div>
          <div class="setting-row">
            <span class="setting-label">{{ t('settings.theme') }}</span>
            <div ref="themeSelectRef" class="theme-select" @mouseleave="clearThemePreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleDropdown">
                <span>{{ previewTheme ? t('settings.theme_' + previewTheme) : t('settings.theme_' + form.theme) }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="themeDropdownOpen" class="theme-select-dropdown">
                <div
                  v-for="opt in themeOptions"
                  :key="opt.value"
                  class="theme-select-option"
                  :class="{ selected: form.theme === opt.value && !previewTheme, previewed: previewTheme === opt.value }"
                  @click.stop="selectTheme(opt.value)"
                  @mouseenter="setThemePreview(opt.value)"
                >
                  {{ t(opt.label) }}
                </div>
              </div>
            </div>
          </div>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.minimize_to_tray" />
            <span class="setting-label">{{ t('settings.minimize_tray') }}</span>
          </label>
        </div>
        <div v-if="settingsTab !== 'data'" class="setting-actions">
          <button class="btn-primary" :disabled="savingSettings" @click="handleSave">
            {{ savingSettings ? t('settings.saving') : t('settings.save') }}
          </button>
        </div>
      </div>
    </div>
  </section>
</template>
