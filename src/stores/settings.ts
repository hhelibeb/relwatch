import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import {
  type AppSettings,
  type CredentialKind,
  DEFAULT_SETTINGS,
  getSettings,
  updateSettings,
  setCredential,
} from '../api/settings'
import { setLocale, t } from '../i18n'
import { applyTheme } from '../composables/useTheme'
import { applyFontScale } from '../composables/useFontScale'
import { setUsageTrackingEnabled } from '../composables/useUsageTracking'
import { normalizeAutoReadDetail } from '../utils'
import { extractErrorMessage } from '../services/feedback'
import { useUiStore } from './ui'

export const useSettingsStore = defineStore('settings', () => {
  const settings = ref<AppSettings>({ ...DEFAULT_SETTINGS })
  const loaded = ref(false)
  const draft = ref<AppSettings>({ ...DEFAULT_SETTINGS })
  const saving = ref(false)

  const aiEnabled = computed(
    () => settings.value.deepseek_enabled && settings.value.deepseek_api_key_set,
  )
  const showImportance = computed(() => settings.value.show_importance)
  const autoReadDetail = computed(() => normalizeAutoReadDetail(settings.value.auto_read_detail))
  const minimizeToTray = computed(() => settings.value.minimize_to_tray)
  const showSourceTypeIcons = computed(() => settings.value.show_source_type_icons)
  const enableUsageStats = computed(() => settings.value.enable_usage_stats)

  const isDraftDirty = computed(() => {
    const keys = Object.keys(DEFAULT_SETTINGS) as (keyof AppSettings)[]
    return keys.some((k) => draft.value[k] !== settings.value[k])
  })

  function applySideEffects(s: AppSettings = settings.value): void {
    setLocale(s.language)
    applyTheme(s.theme)
    applyFontScale(s.font_scale)
    setUsageTrackingEnabled(s.enable_usage_stats)
  }

  function syncDraftFromSettings(source: AppSettings = settings.value): void {
    draft.value = { ...source }
  }

  function setSettings(next: AppSettings): void {
    settings.value = { ...next }
    loaded.value = true
    syncDraftFromSettings(next)
    applySideEffects(next)
  }

  async function loadSettings(): Promise<void> {
    try {
      const next = await getSettings()
      setSettings(next)
    } catch (e: unknown) {
      useUiStore().showToast(t('app.load_failed', extractErrorMessage(e)))
    }
  }

  async function saveSettings(payload: AppSettings): Promise<boolean> {
    const pollChanged = payload.poll_interval_minutes !== settings.value.poll_interval_minutes
    saving.value = true
    try {
      await updateSettings(payload)
      setSettings(payload)
      return pollChanged
    } finally {
      saving.value = false
    }
  }

  const CREDENTIAL_FLAG_KEY: Record<
    CredentialKind,
    'deepseek_api_key_set' | 'github_token_set' | 'youtube_api_key_set' | 'bilibili_cookie_set'
  > = {
    deepseek_api_key: 'deepseek_api_key_set',
    github_token: 'github_token_set',
    youtube_api_key: 'youtube_api_key_set',
    bilibili_cookie: 'bilibili_cookie_set',
  }

  async function saveCredential(kind: CredentialKind, value: string): Promise<void> {
    await setCredential(kind, value)
    const flagKey = CREDENTIAL_FLAG_KEY[kind]
    const isSet = Boolean(value)
    settings.value[flagKey] = isSet
    draft.value[flagKey] = isSet
  }

  let systemThemeMedia: MediaQueryList | null = null
  function watchSystemTheme(): void {
    stopWatchSystemTheme()
    systemThemeMedia = window.matchMedia('(prefers-color-scheme: dark)')
    systemThemeMedia.onchange = () => {
      if (settings.value.theme === 'system') {
        applyTheme('system')
      }
    }
  }

  function stopWatchSystemTheme(): void {
    if (systemThemeMedia) {
      systemThemeMedia.onchange = null
      systemThemeMedia = null
    }
  }

  return {
    settings,
    loaded,
    draft,
    saving,
    aiEnabled,
    showImportance,
    autoReadDetail,
    minimizeToTray,
    showSourceTypeIcons,
    enableUsageStats,
    isDraftDirty,
    applySideEffects,
    syncDraftFromSettings,
    setSettings,
    loadSettings,
    saveSettings,
    saveCredential,
    watchSystemTheme,
    stopWatchSystemTheme,
  }
})
