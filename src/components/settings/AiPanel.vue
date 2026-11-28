<script setup lang="ts">
import { computed } from 'vue'
import type { AppSettings } from '../../api/settings'
import { t } from '../../i18n'

const DEEPSEEK_PROMPT_SUFFIX = '请严格按以下 JSON 格式返回（不要包含其他内容）：\n{"summary":"简短中文摘要","importance":"大|中|小"}'

const form = defineModel<AppSettings>('form', { required: true })
defineProps<{
  dirtyFields: Set<string>
  deepseekApiKey: string
  testingDeepseek: boolean
}>()

const emit = defineEmits<{
  'update:deepseekApiKey': [value: string]
  openAiUsage: []
  testConnection: []
}>()

function resolveChatCompletionUrl(baseUrl: string): string {
  const base = baseUrl.trim().replace(/\/+$/, '')
  const lower = base.toLowerCase()
  if (lower.endsWith('/chat/completions')) return base
  if (lower.endsWith('/v1')) return `${base}/chat/completions`
  return `${base}/v1/chat/completions`
}

const resolvedEndpointPreview = computed(() => {
  const url = form.value.deepseek_base_url.trim()
  if (!url) return ''
  return resolveChatCompletionUrl(url)
})
</script>

<template>
  <div class="settings-form">
    <div class="setting-row">
      <button class="btn-secondary" @click="emit('openAiUsage')">{{ t('settings.ai_usage_stats') }}</button>
      <span class="setting-hint">{{ t('settings.ai_usage_stats_hint') }}</span>
    </div>
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
          :value="deepseekApiKey"
          :placeholder="form.deepseek_api_key_set ? t('settings.api_key_set') : t('settings.api_key_input')"
          class="setting-input"
          @input="emit('update:deepseekApiKey', ($event.target as HTMLInputElement).value)"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_model') || null">{{ t('settings.model') }}</span>
        <input
          type="text"
          v-model="form.deepseek_model"
          :placeholder="t('settings.deepseek_model_placeholder')"
          class="setting-input"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_base_url') || null">{{ t('settings.api_url') }}</span>
        <input
          type="text"
          v-model="form.deepseek_base_url"
          :placeholder="t('settings.deepseek_base_url_placeholder')"
          class="setting-input"
        />
        <span v-if="resolvedEndpointPreview" class="setting-note">{{ t('settings.endpoint_preview', resolvedEndpointPreview) }}</span>
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
        <select v-model="form.deepseek_min_importance" class="select-input setting-select">
          <option value="小">{{ t('settings.importance_any') }}</option>
          <option value="中">{{ t('settings.importance_medium_or_above') }}</option>
          <option value="大">{{ t('settings.importance_high_only') }}</option>
        </select>
      </label>
      <label class="setting-row setting-row-checkbox">
        <input type="checkbox" v-model="form.show_importance" />
        <span class="setting-label" :data-dirty="dirtyFields.has('show_importance') || null">{{ t('settings.show_importance') }}</span>
        <span class="setting-hint">{{ t('settings.show_importance_hint') }}</span>
      </label>
      <div class="setting-row">
        <button class="btn-secondary" :disabled="testingDeepseek" @click="emit('testConnection')">
          {{ testingDeepseek ? t('settings.testing') : t('settings.test_connection') }}
        </button>
        <span class="setting-hint">{{ t('settings.test_connection_hint') }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.setting-textarea {
  width: 100%;
  min-height: 120px;
  resize: vertical;
  font-family: var(--mono-font);
  font-size: var(--fs-meta);
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
  font-family: var(--mono-font);
  font-size: var(--fs-meta);
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
</style>
