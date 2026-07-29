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
} from '../api/settings'
import { openReleaseUrl } from '../api/client'
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

// ── 固定提示词后缀（不可编辑）───────────────────────
const DEEPSEEK_PROMPT_SUFFIX = '请严格按以下 JSON 格式返回（不要包含其他内容）：\n{"summary":"简短中文摘要","importance":"大|中|小"}'

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
      // 守卫：若下拉在 nextTick 执行前已被快速关闭（如同一微任务内再次 toggle），
      // 不应再向 document 注册 outsideClick 监听器，避免监听器泄漏
      if (!themeDropdownOpen.value) return
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
      // 守卫：同 themeDropdownOpen，防止 nextTick 前已关闭导致监听器泄漏
      if (!langDropdownOpen.value) return
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
    // 下拉关闭后把焦点移回触发器，避免 v-if 移除聚焦选项后焦点回退到 body
    nextTick(() => {
      const langTrigger = langSelectRef.value?.querySelector('.theme-select-trigger') as HTMLElement | null
      langTrigger?.focus()
    })
  }, 0)
}

function toggleLangDropdown() {
  langDropdownOpen.value = !langDropdownOpen.value
  if (!langDropdownOpen.value) {
    clearLangPreview()
  } else {
    // 打开时聚焦第一个选项
    nextTick(() => {
      const dropdown = langSelectRef.value?.querySelector('.theme-select-dropdown')
      const firstOption = dropdown?.querySelector('.theme-select-option') as HTMLElement | null
      firstOption?.focus()
    })
  }
}

function handleLangDropdownKeydown(e: KeyboardEvent) {
  // 当下拉关闭时，只处理打开操作
  if (!langDropdownOpen.value) {
    if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      toggleLangDropdown()
    }
    return
  }
  
  const dropdown = langSelectRef.value?.querySelector('.theme-select-dropdown') as HTMLElement | null
  if (!dropdown) return
  
  const options = Array.from(dropdown.querySelectorAll('.theme-select-option')) as HTMLElement[]
  const currentIndex = options.findIndex(opt => opt === document.activeElement)
  
  switch (e.key) {
    case 'ArrowDown': {
      e.preventDefault()
      const nextIndex = currentIndex < options.length - 1 ? currentIndex + 1 : 0
      options[nextIndex]?.focus()
      const nextVal = options[nextIndex]?.getAttribute('data-value')
      if (nextVal) setLangPreview(nextVal)
      break
    }
    case 'ArrowUp': {
      e.preventDefault()
      const prevIndex = currentIndex > 0 ? currentIndex - 1 : options.length - 1
      options[prevIndex]?.focus()
      const prevVal = options[prevIndex]?.getAttribute('data-value')
      if (prevVal) setLangPreview(prevVal)
      break
    }
    case 'Enter':
    case ' ':
      e.preventDefault()
      if (currentIndex >= 0) {
        const option = options[currentIndex] as HTMLElement
        const value = option.getAttribute('data-value')
        if (value) selectLang(value)
      }
      break
    case 'Escape': {
      e.preventDefault()
      langDropdownOpen.value = false
      clearLangPreview()
      const langTrigger = langSelectRef.value?.querySelector('.theme-select-trigger') as HTMLElement | null
      langTrigger?.focus()
      break
    }
    case 'Home': {
      e.preventDefault()
      options[0]?.focus()
      const firstVal = options[0]?.getAttribute('data-value')
      if (firstVal) setLangPreview(firstVal)
      break
    }
    case 'End': {
      e.preventDefault()
      options[options.length - 1]?.focus()
      const lastVal = options[options.length - 1]?.getAttribute('data-value')
      if (lastVal) setLangPreview(lastVal)
      break
    }
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
    // 下拉关闭后把焦点移回触发器，避免 v-if 移除聚焦选项后焦点回退到 body
    nextTick(() => {
      const themeTrigger = themeSelectRef.value?.querySelector('.theme-select-trigger') as HTMLElement | null
      themeTrigger?.focus()
    })
  }, 0)
}

function toggleDropdown() {
  themeDropdownOpen.value = !themeDropdownOpen.value
  if (!themeDropdownOpen.value) {
    clearThemePreview()
  } else {
    // 打开时聚焦第一个选项
    nextTick(() => {
      const dropdown = themeSelectRef.value?.querySelector('.theme-select-dropdown')
      const firstOption = dropdown?.querySelector('.theme-select-option') as HTMLElement | null
      firstOption?.focus()
    })
  }
}

function handleThemeDropdownKeydown(e: KeyboardEvent) {
  // 当下拉关闭时，只处理打开操作
  if (!themeDropdownOpen.value) {
    if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      toggleDropdown()
    }
    return
  }
  
  const dropdown = themeSelectRef.value?.querySelector('.theme-select-dropdown') as HTMLElement | null
  if (!dropdown) return
  
  const options = Array.from(dropdown.querySelectorAll('.theme-select-option')) as HTMLElement[]
  const currentIndex = options.findIndex(opt => opt === document.activeElement)
  
  switch (e.key) {
    case 'ArrowDown': {
      e.preventDefault()
      const nextIndex = currentIndex < options.length - 1 ? currentIndex + 1 : 0
      options[nextIndex]?.focus()
      const nextVal = options[nextIndex]?.getAttribute('data-value')
      if (nextVal) setThemePreview(nextVal)
      break
    }
    case 'ArrowUp': {
      e.preventDefault()
      const prevIndex = currentIndex > 0 ? currentIndex - 1 : options.length - 1
      options[prevIndex]?.focus()
      const prevVal = options[prevIndex]?.getAttribute('data-value')
      if (prevVal) setThemePreview(prevVal)
      break
    }
    case 'Enter':
    case ' ':
      e.preventDefault()
      if (currentIndex >= 0) {
        const option = options[currentIndex] as HTMLElement
        const value = option.getAttribute('data-value')
        if (value) selectTheme(value)
      }
      break
    case 'Escape': {
      e.preventDefault()
      themeDropdownOpen.value = false
      clearThemePreview()
      const themeTrigger = themeSelectRef.value?.querySelector('.theme-select-trigger') as HTMLElement | null
      themeTrigger?.focus()
      break
    }
    case 'Home': {
      e.preventDefault()
      options[0]?.focus()
      const firstVal = options[0]?.getAttribute('data-value')
      if (firstVal) setThemePreview(firstVal)
      break
    }
    case 'End': {
      e.preventDefault()
      options[options.length - 1]?.focus()
      const lastVal = options[options.length - 1]?.getAttribute('data-value')
      if (lastVal) setThemePreview(lastVal)
      break
    }
  }
}

async function handleSave() {
  savingSettings.value = true
  try {
    const s = form
    // 验证提示词
    if (s.deepseek_prompt && !s.deepseek_prompt.includes('{}')) {
      showToast(t('settings.deepseek_prompt_validate_failed'))
      savingSettings.value = false
      return
    }
    // 先验证提示词、持久化主设置，再写敏感凭据：保证 updateSettings 失败时凭据不会被误写入，
    // 避免“凭据已持久化但用户以为整体保存失败”的非原子状态。
    setLocale(form.language)
    await updateSettings({
      pollIntervalMinutes: s.poll_interval_minutes,
      proxyMode: s.proxy_mode,
      proxyUrl: s.proxy_url.trim(),
      autoStart: s.auto_start,
      minimizeToTray: s.minimize_to_tray,
      logRetentionDays: s.log_retention_days,
      deepseekEnabled: s.deepseek_enabled,
      deepseekModel: s.deepseek_model.trim() || 'deepseek-v4-flash',
      deepseekBaseUrl: s.deepseek_base_url.trim() || 'https://api.deepseek.com',
      deepseekProxyBypass: s.deepseek_proxy_bypass,
      deepseekPrompt: s.deepseek_prompt,
      deepseekMinImportance: s.deepseek_min_importance,
      deepseekTranslateRelease: s.deepseek_translate_release,

      checkPrereleases: s.check_prereleases,
      fetchHistory: s.fetch_history,
      fetchHistoryCount: s.fetch_history_count ?? 1,
      language: s.language,
      theme: s.theme,
      showSourceTypeIcons: s.show_source_type_icons,
    })
    // 主设置持久化成功后再写凭据；若凭据写入失败，走外层 catch 提示 save_failed，
    // 此时主设置已存、凭据未存，用户可重试凭据。
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
    showToast(t('settings.saved'))
    const pollChanged = form.poll_interval_minutes !== prevPollInterval.value
    if (pollChanged) prevPollInterval.value = form.poll_interval_minutes
    emit('update', pollChanged)
  } catch (e: unknown) {
    // updateSettings 失败时回滚 UI 语言到已持久化的语言，避免 UI 与后端不一致
    setLocale(props.settings.language)
    showToast(t('settings.save_failed') + (e instanceof Error ? e.message : String(e)))
  } finally {
    savingSettings.value = false
    // dirty 标记会在 props.settings 更新后自动清除
  }
}

// ── 脏标记 ────────────────────────────────────────────

const trackedKeys = [
  'poll_interval_minutes', 'proxy_mode', 'proxy_url', 'auto_start', 'minimize_to_tray',
  'log_retention_days', 'check_prereleases', 'fetch_history',
  'fetch_history_count', 'deepseek_enabled', 'deepseek_model',
  'deepseek_base_url', 'deepseek_proxy_bypass', 'deepseek_prompt',
  'deepseek_min_importance', 'deepseek_translate_release', 'language', 'theme', 'show_source_type_icons',
] as const

const dirtyFields = computed(() => {
  const dirty = new Set<string>()
  for (const key of trackedKeys) {
    if ((form as unknown as Record<string, unknown>)[key] !== (props.settings as unknown as Record<string, unknown>)[key]) {
      dirty.add(key)
    }
  }
  if (deepseekApiKey.value) dirty.add('deepseek_api_key')
  if (githubToken.value) dirty.add('github_token')
  return dirty
})

const dirtyCount = computed(() => dirtyFields.value.size)

const dirtyByTab = computed(() => {
  const f = dirtyFields.value
  return {
    general: ['auto_start', 'poll_interval_minutes', 'proxy_mode', 'proxy_url', 'github_token', 'log_retention_days', 'check_prereleases', 'fetch_history', 'fetch_history_count'].filter(k => f.has(k)).length,
    appearance: ['language', 'theme', 'minimize_to_tray', 'show_source_type_icons'].filter(k => f.has(k)).length,
    ai: ['deepseek_enabled', 'deepseek_api_key', 'deepseek_model', 'deepseek_base_url', 'deepseek_proxy_bypass', 'deepseek_prompt', 'deepseek_min_importance', 'deepseek_translate_release'].filter(k => f.has(k)).length,
  }
})

function discardChanges() {
  const langDirty = dirtyFields.value.has('language')
  const themeDirty = dirtyFields.value.has('theme')
  Object.assign(form, props.settings)
  if (langDirty) setLocale(form.language)
  // 使用 clearThemePreview 清除预览状态（previewTheme 置 null），避免残留“预览中”语义
  // 导致下次打开下拉时 .previewed 类错误高亮当前主题选项
  if (themeDirty) clearThemePreview()
  deepseekApiKey.value = ''
  githubToken.value = ''
}

async function handleTestDeepseek() {
  testingDeepseek.value = true
  try {
    // 传入表单当前值（含未保存修改）测试：API Key 留空时后端回退到已保存的 key
    const msg = await testDeepseekConnection({
      model: form.deepseek_model.trim(),
      baseUrl: form.deepseek_base_url.trim(),
      apiKey: deepseekApiKey.value,
      proxyBypass: form.deepseek_proxy_bypass,
      proxyUrl: form.proxy_url.trim(),
      proxyMode: form.proxy_mode,
    })
    await message(msg, { title: t('settings.deepseek_test_title'), kind: 'info' })
  } catch (e: unknown) {
    await message(t('settings.connect_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.deepseek_test_title'), kind: 'error' })
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
    // 后端用 `err.backup_cancelled_export` 稳定 key 表示用户取消；invokeI18n 已将其翻译为
    // t('err.backup_cancelled_export')，两侧同走 i18n，故比较结果与 UI 语言一致、不依赖中文子串。
    if (msg === t('err.backup_cancelled_export')) {
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
    // 同 export：用 `err.backup_cancelled_import` 稳定 key 判定用户取消，不依赖中文子串。
    if (msg === t('err.backup_cancelled_import')) {
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
        <button :class="{ active: settingsTab === 'general' }" @click="settingsTab = 'general'">{{ t('settings.general') }}<span v-if="dirtyByTab.general" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'appearance' }" @click="settingsTab = 'appearance'">{{ t('settings.appearance') }}<span v-if="dirtyByTab.appearance" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'ai' }" @click="settingsTab = 'ai'">{{ t('settings.ai') }}<span v-if="dirtyByTab.ai" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'data' }" @click="settingsTab = 'data'">{{ t('settings.data') }}</button>
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
        <div v-if="dirtyCount > 0" class="settings-banner">
          <span class="settings-banner-text">{{ t('settings.unsaved_banner', String(dirtyCount)) }}</span>
          <div class="settings-banner-actions">
            <button class="btn-secondary" @click="discardChanges">{{ t('settings.discard') }}</button>
            <button class="btn-primary" @click="handleSave">{{ t('settings.save') }}</button>
          </div>
        </div>
        <div v-if="settingsTab === 'general'" class="settings-form">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.auto_start" />
            <span class="setting-label" :data-dirty="dirtyFields.has('auto_start') || null">{{ t('settings.auto_start') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('poll_interval_minutes') || null">{{ t('settings.poll_interval') }}</span>
            <input
              type="number"
              v-model.number="form.poll_interval_minutes"
              min="5"
              max="1440"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('proxy_mode') || null">{{ t('settings.proxy_mode') }}</span>
            <select v-model="form.proxy_mode" class="setting-input setting-input-narrow" style="width:calc(14ch * 1.25)">
              <option value="none">{{ t('settings.proxy_none') }}</option>
              <option value="system">{{ t('settings.proxy_system') }}</option>
              <option value="custom">{{ t('settings.proxy_custom') }}</option>
            </select>
          </label>
          <label class="setting-row" v-if="form.proxy_mode === 'custom'">
            <span class="setting-label" :data-dirty="dirtyFields.has('proxy_url') || null">{{ t('settings.proxy') }}</span>
            <input
              type="text"
              v-model="form.proxy_url"
              :placeholder="t('settings.proxy_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('github_token') || null">{{ t('settings.github_token') }}</span>
            <input
              type="password"
              v-model="githubToken"
              :placeholder="form.github_token_set ? t('settings.github_token_set') : t('settings.github_token_input')"
              class="setting-input"
            />
            <span class="setting-note">{{ t('settings.github_token_note') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('log_retention_days') || null">{{ t('settings.log_retention') }}</span>
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
            <span class="setting-label" :data-dirty="dirtyFields.has('check_prereleases') || null">{{ t('settings.check_prereleases') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.fetch_history" />
            <span class="setting-label" :data-dirty="dirtyFields.has('fetch_history') || null">{{ t('settings.fetch_history') }}</span>
          </label>
          <label class="setting-row" v-if="form.fetch_history">
            <span class="setting-label" :data-dirty="dirtyFields.has('fetch_history_count') || null">{{ t('settings.fetch_history_count') }}</span>
            <input
              type="number"
              v-model.number="form.fetch_history_count"
              min="0"
              max="100"
              class="setting-input setting-input-narrow"
            />
            <span class="setting-note">{{ t('settings.fetch_history_count_hint') }}</span>
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
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_enabled') || null">{{ t('settings.enable_ai') }}</span>
          </label>
          <template v-if="form.deepseek_enabled">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_proxy_bypass" />
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_proxy_bypass') || null">{{ t('settings.deepseek_proxy_bypass') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_translate_release" />
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_translate_release') || null">{{ t('settings.translate_release') }}</span>
            <span class="setting-hint">{{ t('settings.translate_release_desc') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_api_key') || null">{{ t('settings.api_key') }}</span>
            <input
              type="password"
              v-model="deepseekApiKey"
              :placeholder="form.deepseek_api_key_set ? t('settings.api_key_set') : t('settings.api_key_input')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_model') || null">{{ t('settings.model') }}</span>
            <input
              type="text"
              v-model="form.deepseek_model"
              placeholder="deepseek-v4-flash"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_base_url') || null">{{ t('settings.api_url') }}</span>
            <input
              type="text"
              v-model="form.deepseek_base_url"
              placeholder="https://api.deepseek.com"
              class="setting-input"
            />
          </label>
          <label class="setting-row setting-row-textarea">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_prompt') || null">{{ t('settings.deepseek_prompt') }}</span>
            <textarea
              v-model="form.deepseek_prompt"
              :placeholder="t('settings.deepseek_prompt_placeholder')"
              class="setting-input setting-textarea"
              rows="10"
            />
            <div class="setting-prompt-fixed">
              <code>{{ DEEPSEEK_PROMPT_SUFFIX }}</code>
            </div>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_min_importance') || null">{{ t('settings.notify_threshold') }}</span>
            <select v-model="form.deepseek_min_importance" class="setting-input setting-input-narrow" style="width:calc(14ch * 1.25)">
              <option value="小">{{ t('settings.importance_any') }}</option>
              <option value="中">{{ t('settings.importance_medium_or_above') }}</option>
              <option value="大">{{ t('settings.importance_high_only') }}</option>
            </select>
          </label>

          <div class="setting-row">
            <button class="btn-secondary" :disabled="testingDeepseek" @click="handleTestDeepseek">
              {{ testingDeepseek ? t('settings.testing') : t('settings.test_connection') }}
            </button>
            <span class="setting-hint">{{ t('settings.test_connection_hint') }}</span>
          </div>
          </template>
        </div>
        <div v-if="settingsTab === 'appearance'" class="settings-form">
          <div class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('language') || null">{{ t('settings.language') }}<svg class="label-icon"><use href="/icons.svg#language-icon"/></svg></span>
            <div ref="langSelectRef" class="theme-select" @mouseleave="clearLangPreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleLangDropdown" @keydown="handleLangDropdownKeydown" :aria-expanded="langDropdownOpen" aria-haspopup="listbox">
                <span>{{ previewLang ? languages.find(l => l.value === previewLang)?.label : languages.find(l => l.value === form.language)?.label }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="langDropdownOpen" class="theme-select-dropdown" role="listbox" @keydown="handleLangDropdownKeydown">
                <div
                  v-for="lang in languages"
                  :key="lang.value"
                  class="theme-select-option"
                  :class="{ selected: form.language === lang.value && !previewLang, previewed: previewLang === lang.value }"
                  :data-value="lang.value"
                  tabindex="-1"
                  role="option"
                  :aria-selected="form.language === lang.value"
                  @click.stop="selectLang(lang.value)"
                  @mouseenter="setLangPreview(lang.value)"
                >
                  {{ lang.label }}
                </div>
              </div>
            </div>
          </div>
          <div class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('theme') || null">{{ t('settings.theme') }}</span>
            <div ref="themeSelectRef" class="theme-select" @mouseleave="clearThemePreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleDropdown" @keydown="handleThemeDropdownKeydown" :aria-expanded="themeDropdownOpen" aria-haspopup="listbox">
                <span>{{ previewTheme ? t('settings.theme_' + previewTheme) : t('settings.theme_' + form.theme) }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="themeDropdownOpen" class="theme-select-dropdown" role="listbox" @keydown="handleThemeDropdownKeydown">
                <div
                  v-for="opt in themeOptions"
                  :key="opt.value"
                  class="theme-select-option"
                  :class="{ selected: form.theme === opt.value && !previewTheme, previewed: previewTheme === opt.value }"
                  :data-value="opt.value"
                  tabindex="-1"
                  role="option"
                  :aria-selected="form.theme === opt.value"
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
            <span class="setting-label" :data-dirty="dirtyFields.has('minimize_to_tray') || null">{{ t('settings.minimize_tray') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.show_source_type_icons" />
            <span class="setting-label" :data-dirty="dirtyFields.has('show_source_type_icons') || null">{{ t('settings.show_source_type_icons') }}</span>
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
<style scoped>
/* 设置 */
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
  font-size: 13px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
  transition: background 0.15s, color 0.15s;
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
  background: var(--bg-subtle);
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
  font-size: 11px;
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
  transition: color 0.15s, background 0.15s;
}

.version-github-btn:hover {
  color: var(--text);
  background: var(--bg-subtle);
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

.settings-form {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 2px 0 24px;
  background: transparent;
}

.setting-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.setting-label {
  font-size: 13px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.setting-label[data-dirty]::after {
  content: '●';
  color: var(--warning);
  font-size: 10px;
  margin-left: 2px;
}

.label-icon {
  width: 14px;
  height: 14px;
  color: var(--text-muted);
}

.setting-input {
  padding: 8px 12px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.setting-input-narrow {
  width: 14ch;
}

.setting-textarea {
  width: 100%;
  min-height: 120px;
  resize: vertical;
  font-family: 'Consolas', 'Courier New', monospace;
  font-size: 0.85em;
  line-height: 1.5;
}

.setting-row-textarea {
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
}

.setting-row-textarea .setting-label {
  margin-bottom: 2px;
}

.setting-prompt-fixed {
  width: 100%;
  padding: 8px 12px;
  background: var(--bg-subtle);
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-sm);
  font-family: 'Consolas', 'Courier New', monospace;
  font-size: 0.85em;
  line-height: 1.5;
  color: var(--text-muted);
  user-select: none;
  cursor: not-allowed;
  white-space: pre-wrap;
  opacity: 0.85;
}

.setting-prompt-fixed code {
  display: block;
  background: transparent;
  padding: 0;
  color: inherit;
}

.setting-input {
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

.setting-input:focus {
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}

.setting-note {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.4;
}

select.setting-input {
  appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'%3E%3Cpath fill='none' stroke='%239ca3af' stroke-width='1.5' d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 10px center;
  padding-right: 28px;
}

.setting-row-checkbox {
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.settings-form .btn-primary {
  margin-left: 0;
  align-self: flex-start;
}

.setting-divider {
  border: none;
  border-top: 1px solid var(--border);
  margin: 4px 0;
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
  font-size: 13px;
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
  font-size: 12px;
  white-space: nowrap;
}

.setting-section-title {
  font-size: 15px;
  font-weight: 600;
  margin: 0;
}

.setting-section-sep {
  border: none;
  border-top: 1px solid var(--border);
  margin: 16px 0;
}

.setting-section-desc {
  font-size: 12px;
  color: var(--text-muted);
  margin: 4px 0 12px;
  line-height: 1.5;
}

.setting-hint {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.5;
  flex-basis: 100%;
  width: 100%;
  margin-left: 0;
  margin-top: -2px;
}

.backup-actions {
  display: flex;
  gap: 10px;
  margin-top: 8px;
}

.setting-actions {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-top: 16px;
}

.btn-secondary {
  align-self: flex-start;
  padding: 6px 16px;
  background: var(--surface);
  color: var(--text);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  transition: background 0.12s ease, border-color 0.12s ease;
}

.btn-secondary:hover {
  background: var(--bg-subtle);
  border-color: var(--text-faint);
}

/* ── 自定义主题下拉选择器 ───────────────────────── */
.theme-select {
  position: relative;
  width: 18ch;
}

.theme-select-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  cursor: pointer;
  text-align: left;
  font-size: 13px;
}

.theme-select-arrow {
  flex-shrink: 0;
  margin-left: 4px;
  transition: transform 0.2s;
}

.theme-select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  width: 100%;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  z-index: 100;
  overflow: hidden;
}

.theme-select-option {
  padding: 8px 12px;
  font-size: 13px;
  cursor: pointer;
  color: var(--text);
  transition: background 0.1s;
}

.theme-select-option:hover,
.theme-select-option.previewed {
  background: var(--bg-subtle);
  color: var(--text);
}

.theme-select-option.selected {
  font-weight: 600;
}
</style>
