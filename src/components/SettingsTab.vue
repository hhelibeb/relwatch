<script setup lang="ts">
import { ref, inject } from 'vue'
import { message } from '@tauri-apps/plugin-dialog'
import { version } from '../../package.json'
import {
  type AppSettings,
  updateSettings,
  setDeepseekApiKey,
  setGithubToken,
  testDeepseekConnection,
} from '../api'
import { t, setLocale, languages } from '../i18n'

const props = defineProps<{ settings: AppSettings }>()
const emit = defineEmits<{ update: [pollIntervalChanged: boolean] }>()
const showToast = inject<(msg: string) => void>('showToast')!

const settingsTab = ref<'general' | 'ai'>('general')
const savingSettings = ref(false)
const deepseekApiKey = ref('')
const githubToken = ref('')
const testingDeepseek = ref(false)
const prevPollInterval = ref(props.settings.poll_interval_minutes)

async function handleSave() {
  savingSettings.value = true
  try {
    await updateSettings(
      props.settings.poll_interval_minutes,
      props.settings.proxy_url.trim(),
      props.settings.minimize_to_tray,
      props.settings.log_retention_days,
      props.settings.deepseek_enabled,
      props.settings.deepseek_model.trim() || 'deepseek-v4-flash',
      props.settings.deepseek_base_url.trim() || 'https://api.deepseek.com',
      props.settings.deepseek_proxy_enabled,
      props.settings.check_prereleases,
      props.settings.language,
    )
    if (deepseekApiKey.value) {
      await setDeepseekApiKey(deepseekApiKey.value)
      deepseekApiKey.value = ''
      props.settings.deepseek_api_key_set = true
    }
    if (githubToken.value) {
      await setGithubToken(githubToken.value)
      githubToken.value = ''
      props.settings.github_token_set = true
    }
    setLocale(props.settings.language)
    showToast(t('settings.saved'))
    const pollChanged = props.settings.poll_interval_minutes !== prevPollInterval.value
    if (pollChanged) prevPollInterval.value = props.settings.poll_interval_minutes
    emit('update', pollChanged)
  } catch (e: any) {
    showToast(t('settings.save_failed') + (e?.toString?.() ?? String(e)))
  } finally {
    savingSettings.value = false
  }
}

async function handleTestDeepseek() {
  testingDeepseek.value = true
  try {
    const msg = await testDeepseekConnection()
    await message(msg, { title: 'DeepSeek Connection Test', kind: 'info' })
  } catch (e: any) {
    await message(t('settings.connect_failed') + (e?.toString?.() ?? String(e)), { title: 'DeepSeek Connection Test', kind: 'error' })
  } finally {
    testingDeepseek.value = false
  }
}
</script>

<template>
  <section class="tab-content">
    <div class="settings-layout">
      <aside class="settings-sidebar">
        <button :class="{ active: settingsTab === 'general' }" @click="settingsTab = 'general'">{{ t('settings.general') }}</button>
        <button :class="{ active: settingsTab === 'ai' }" @click="settingsTab = 'ai'">{{ t('settings.ai') }}</button>
        <span class="version-text">v{{ version }}</span>
      </aside>
      <div class="settings-main">
        <div v-if="settingsTab === 'general'" class="settings-form">
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.language') }}<svg class="label-icon"><use href="/icons.svg#language-icon"/></svg></span>
            <select v-model="props.settings.language" class="setting-input setting-input-narrow">
              <option v-for="lang in languages" :key="lang.value" :value="lang.value">{{ lang.label }}</option>
            </select>
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.poll_interval') }}</span>
            <input
              type="number"
              v-model.number="props.settings.poll_interval_minutes"
              min="5"
              max="1440"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.proxy') }}</span>
            <input
              type="text"
              v-model="props.settings.proxy_url"
              :placeholder="t('settings.proxy_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.github_token') }}</span>
            <input
              type="password"
              v-model="githubToken"
              :placeholder="props.settings.github_token_set ? t('settings.github_token_set') : t('settings.github_token_input')"
              class="setting-input"
            />
            <span class="setting-note">{{ t('settings.github_token_note') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="props.settings.minimize_to_tray" />
            <span class="setting-label">{{ t('settings.minimize_tray') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.log_retention') }}</span>
            <input
              type="number"
              v-model.number="props.settings.log_retention_days"
              min="0"
              max="3650"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="props.settings.check_prereleases" />
            <span class="setting-label">{{ t('settings.check_prereleases') }}</span>
          </label>
        </div>
        <div v-if="settingsTab === 'ai'" class="settings-form">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="props.settings.deepseek_enabled" />
            <span class="setting-label">{{ t('settings.enable_ai') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.api_key') }}</span>
            <input
              type="password"
              v-model="deepseekApiKey"
              :placeholder="props.settings.deepseek_api_key_set ? t('settings.api_key_set') : t('settings.api_key_input')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.model') }}</span>
            <input
              type="text"
              v-model="props.settings.deepseek_model"
              placeholder="deepseek-v4-flash"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('settings.api_url') }}</span>
            <input
              type="text"
              v-model="props.settings.deepseek_base_url"
              placeholder="https://api.deepseek.com"
              class="setting-input"
            />
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="props.settings.deepseek_proxy_enabled" />
            <span class="setting-label">{{ t('settings.use_proxy') }}</span>
          </label>
          <div class="setting-row">
            <button class="btn-secondary" :disabled="testingDeepseek" @click="handleTestDeepseek">
              {{ testingDeepseek ? t('settings.testing') : t('settings.test_connection') }}
            </button>
          </div>
        </div>
        <div class="setting-actions">
          <button class="btn-primary" :disabled="savingSettings" @click="handleSave">
            {{ savingSettings ? t('settings.saving') : t('settings.save') }}
          </button>
        </div>
      </div>
    </div>
  </section>
</template>
