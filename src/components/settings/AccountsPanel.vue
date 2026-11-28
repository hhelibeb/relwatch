<script setup lang="ts">
import type { AppSettings } from '../../api/settings'
import { t } from '../../i18n'

defineProps<{
  form: AppSettings
  dirtyFields: Set<string>
  githubToken: string
  youtubeApiKey: string
  bilibiliCookie: string
  biliLoginBusy: boolean
}>()

const emit = defineEmits<{
  'update:githubToken': [value: string]
  'update:youtubeApiKey': [value: string]
  'update:bilibiliCookie': [value: string]
  bilibiliLogin: []
  clearBilibiliCookie: []
}>()
</script>

<template>
  <div class="settings-form">
    <label class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('github_token') || null">{{ t('settings.github_token') }}</span>
      <input
        type="password"
        :value="githubToken"
        :placeholder="form.github_token_set ? t('settings.github_token_set') : t('settings.github_token_input')"
        class="setting-input"
        @input="emit('update:githubToken', ($event.target as HTMLInputElement).value)"
      />
      <span class="setting-note">{{ t('settings.github_token_note') }}</span>
    </label>
    <label class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('youtube_api_key') || null">{{ t('settings.youtube_api_key') }}</span>
      <input
        type="password"
        :value="youtubeApiKey"
        :placeholder="form.youtube_api_key_set ? t('settings.youtube_api_key_set') : t('settings.youtube_api_key_input')"
        class="setting-input"
        @input="emit('update:youtubeApiKey', ($event.target as HTMLInputElement).value)"
      />
      <span class="setting-note">{{ t('settings.youtube_api_key_note') }}</span>
    </label>
    <label class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('bilibili_cookie') || null">{{ t('settings.bilibili_cookie') }}</span>
      <div class="bili-cookie-row">
        <input
          type="password"
          :value="bilibiliCookie"
          :placeholder="form.bilibili_cookie_set ? t('settings.bilibili_cookie_set') : t('settings.bilibili_cookie_input')"
          class="setting-input"
          @input="emit('update:bilibiliCookie', ($event.target as HTMLInputElement).value)"
        />
        <button class="btn-secondary bili-login-btn" :disabled="biliLoginBusy" @click="emit('bilibiliLogin')">
          {{ biliLoginBusy ? t('settings.bilibili_login_waiting') : t('settings.bilibili_login_btn') }}
        </button>
        <button
          v-if="form.bilibili_cookie_set && !bilibiliCookie"
          class="btn-secondary bili-clear-btn"
          @click="emit('clearBilibiliCookie')"
        >
          {{ t('settings.bilibili_cookie_clear') }}
        </button>
      </div>
      <span class="setting-note">{{ t('settings.bilibili_cookie_note') }}</span>
    </label>
  </div>
</template>

<style scoped>
.bili-cookie-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.bili-cookie-row .setting-input {
  flex: 1;
  min-width: 0;
}

.bili-login-btn {
  white-space: nowrap;
  flex-shrink: 0;
}
</style>
