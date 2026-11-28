<script setup lang="ts">
import type { AppSettings } from '../../api/settings'
import { t } from '../../i18n'

const form = defineModel<AppSettings>('form', { required: true })
defineProps<{
  dirtyFields: Set<string>
}>()
</script>

<template>
  <div class="settings-form">
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
      <select v-model="form.proxy_mode" class="select-input setting-select">
        <option value="none">{{ t('settings.proxy_none') }}</option>
        <option value="system">{{ t('settings.proxy_system') }}</option>
        <option value="custom">{{ t('settings.proxy_custom') }}</option>
      </select>
    </label>
    <label v-if="form.proxy_mode === 'custom'" class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('proxy_url') || null">{{ t('settings.proxy') }}</span>
      <input
        type="text"
        v-model="form.proxy_url"
        :placeholder="t('settings.proxy_placeholder')"
        class="setting-input"
      />
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
    <label v-if="form.fetch_history" class="setting-row">
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
    <label class="setting-row setting-row-checkbox">
      <input type="checkbox" v-model="form.enable_usage_stats" />
      <span class="setting-label" :data-dirty="dirtyFields.has('enable_usage_stats') || null">{{ t('settings.enable_usage_stats') }}</span>
      <span class="setting-hint">{{ t('settings.enable_usage_stats_hint') }}</span>
    </label>
    <label class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('auto_read_detail') || null">{{ t('settings.auto_read_detail') }}</span>
      <select v-model="form.auto_read_detail" class="select-input setting-select">
        <option value="off">{{ t('settings.auto_read_detail_off') }}</option>
        <option value="open">{{ t('settings.auto_read_detail_open') }}</option>
        <option value="navigate">{{ t('settings.auto_read_detail_navigate') }}</option>
      </select>
      <span class="setting-hint">{{ t('settings.auto_read_detail_hint') }}</span>
    </label>
  </div>
</template>
