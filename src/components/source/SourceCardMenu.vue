<script setup lang="ts">
import { type Source, getSourceTypeDef, parseYtConfig, readConfigFlag } from '../../api/sources'
import { t } from '../../i18n'

const props = defineProps<{
  source: Source
  open: boolean
  unreadCount: number
  agentEnabled: boolean
  onTriggerClick: (e: MouseEvent) => void
  onTriggerKeydown: (e: KeyboardEvent) => void
  onDropdownKeydown: (e: KeyboardEvent) => void
}>()

const emit = defineEmits<{
  ytConfigToggle: [key: 'videos' | 'live', value: boolean]
  flagChange: [key: 'fetch_history' | 'check_prereleases', value: string]
  markAllRead: []
  muteToggle: []
  sendToAgent: []
  remove: []
}>()

function sourceYtConfig(source: Source) {
  return parseYtConfig(source.config)
}

function sourceFlagValue(key: 'fetch_history' | 'check_prereleases'): string {
  const v = readConfigFlag(props.source.config, key)
  return v === undefined ? 'global' : v ? 'on' : 'off'
}
</script>

<template>
  <div class="dropdown-more">
    <button
      type="button"
      class="btn-icon-action btn-more"
      :aria-expanded="open"
      aria-haspopup="menu"
      :title="t('source.more')"
      @click.stop="onTriggerClick"
      @keydown="onTriggerKeydown"
    >
      <svg><use href="/icons.svg#more-icon" /></svg>
    </button>
    <div v-if="open" class="dropdown-panel dropdown-more-panel" role="menu" @click.stop @keydown="onDropdownKeydown">
      <template v-if="getSourceTypeDef(source.source_type)?.hasConfigInput">
        <div class="dropdown-section-label">{{ t('source.subscribe_title') }}</div>
        <label class="yt-menu-option">
          <input
            type="checkbox"
            :checked="sourceYtConfig(source).videos"
            @change="emit('ytConfigToggle', 'videos', ($event.target as HTMLInputElement).checked)"
          />{{ t('source.subscribe_videos') }}
        </label>
        <label class="yt-menu-option">
          <input
            type="checkbox"
            :checked="sourceYtConfig(source).live"
            @change="emit('ytConfigToggle', 'live', ($event.target as HTMLInputElement).checked)"
          />{{ t('source.subscribe_live') }}
        </label>
        <label class="yt-menu-option yt-subscribe-disabled" :title="t('source.posts_unsupported')">
          <input type="checkbox" :checked="false" disabled />{{ t('source.subscribe_posts') }}
        </label>
        <div class="sort-dropdown-divider"></div>
      </template>
      <div class="dropdown-section-label">{{ t('source.settings_title') }}</div>
      <label class="source-setting-row">
        <span class="source-setting-label">{{ t('source.fetch_history') }}</span>
        <select
          class="select-input source-setting-input"
          :value="sourceFlagValue('fetch_history')"
          @click.stop
          @change="emit('flagChange', 'fetch_history', ($event.target as HTMLSelectElement).value)"
        >
          <option value="global">{{ t('source.follow_global') }}</option>
          <option value="on">{{ t('source.switch_on') }}</option>
          <option value="off">{{ t('source.switch_off') }}</option>
        </select>
      </label>
      <label v-if="getSourceTypeDef(source.source_type)?.supportsPrerelease" class="source-setting-row">
        <span class="source-setting-label">{{ t('source.check_prereleases') }}</span>
        <select
          class="select-input source-setting-input"
          :value="sourceFlagValue('check_prereleases')"
          @click.stop
          @change="emit('flagChange', 'check_prereleases', ($event.target as HTMLSelectElement).value)"
        >
          <option value="global">{{ t('source.follow_global') }}</option>
          <option value="on">{{ t('source.switch_on') }}</option>
          <option value="off">{{ t('source.switch_off') }}</option>
        </select>
      </label>
      <div class="sort-dropdown-divider"></div>
      <button v-if="unreadCount > 0" type="button" role="menuitem" class="dropdown-item" @click="emit('markAllRead')">
        <span class="dropdown-icon"><svg><use href="/icons.svg#check-double-icon" /></svg></span>
        {{ t('source.mark_all_read', String(unreadCount)) }}
      </button>
      <button
        type="button"
        role="menuitem"
        class="dropdown-item"
        :disabled="!source.enabled"
        :title="!source.enabled ? t('source.mute_disabled_tip') : ''"
        @click="emit('muteToggle')"
      >
        <span class="dropdown-icon"><svg><use :href="source.muted ? '/icons.svg#bell-icon' : '/icons.svg#bell-off-icon'" /></svg></span>
        {{ source.muted ? t('source.unmute') : t('source.mute') }}
      </button>
      <button v-if="agentEnabled" type="button" role="menuitem" class="dropdown-item" @click="emit('sendToAgent')">
        <span class="dropdown-icon"><svg><use href="/icons.svg#agent-icon" /></svg></span>
        {{ t('agent.send_to') }}
      </button>
      <button type="button" role="menuitem" class="dropdown-item dropdown-item-danger" @click="emit('remove')">
        <span class="dropdown-icon"><svg><use href="/icons.svg#trash-icon" /></svg></span>
        {{ t('source.delete') }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.dropdown-more {
  position: relative;
  display: inline-flex;
}

.dropdown-more-panel {
  top: 100%;
  right: 0;
  margin-top: 4px;
  min-width: 140px;
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.dropdown-item {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 8px 14px;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: var(--fs-body);
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out);
  white-space: nowrap;
}

.dropdown-item:hover:not(:disabled) {
  background: var(--hover-surface);
}

.dropdown-item:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.dropdown-icon {
  font-size: var(--fs-body);
  line-height: 1;
}

.dropdown-icon svg {
  width: 14px;
  height: 14px;
}

.dropdown-item-danger {
  color: var(--danger);
}

.dropdown-item-danger:hover:not(:disabled) {
  background: var(--danger-soft-bg);
}

.sort-dropdown-divider {
  height: 1px;
  margin: 4px 2px;
  background: var(--border);
}

.dropdown-section-label {
  padding: 7px 14px 3px;
  font-size: var(--fs-caption);
  color: var(--text-muted);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.yt-menu-option {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 14px;
  font-size: var(--fs-body);
  color: var(--text);
  cursor: pointer;
  white-space: nowrap;
  user-select: none;
}

.yt-menu-option:hover {
  background: var(--hover-surface);
}

.yt-menu-option input[type="checkbox"] {
  width: 14px;
  height: 14px;
  cursor: pointer;
  accent-color: var(--primary);
}

.yt-menu-option.yt-subscribe-disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.source-setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 4px 14px;
  font-size: var(--fs-body);
  color: var(--text);
  white-space: nowrap;
}

.source-setting-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.select-input.source-setting-input {
  width: auto;
  min-width: calc(13ch * 1.1);
  padding: 3px 20px 3px 8px;
  background-position: right 6px center;
  flex: none;
  font-size: var(--fs-meta);
}

.btn-icon-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out);
  flex-shrink: 0;
}

.btn-icon-action svg {
  width: 16px;
  height: 16px;
}

.btn-icon-action:hover:not(:disabled) {
  background: var(--hover-surface-nested);
  color: var(--text);
}
</style>
