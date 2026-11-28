<script setup lang="ts">
import { type Source, getSourceTypeDef, sourceDisplayName } from '../../api/sources'
import { translateError } from '../../api/client'
import { t } from '../../i18n'
import { formatDate } from '../../utils'
import SourceCardMenu from './SourceCardMenu.vue'

defineProps<{
  source: Source
  highlighted: boolean
  selectionMode: boolean
  selected: boolean
  showSourceTypeIcons: boolean
  unreadCount: number
  polling: boolean
  checkingId: number | null
  moreOpen: boolean
  agentEnabled: boolean
  agentPanelOpen: boolean
  onMoreTriggerClick: (e: MouseEvent) => void
  onMoreTriggerKeydown: (e: KeyboardEvent) => void
  onMoreDropdownKeydown: (e: KeyboardEvent) => void
}>()

const emit = defineEmits<{
  dragStart: [e: DragEvent]
  checkboxClick: [e: MouseEvent]
  toggleSelect: []
  openUrl: []
  contextMenuUrl: [e: MouseEvent]
  openReleases: []
  openUnreadReleases: []
  showHealthTooltip: [e: MouseEvent]
  hideHealthTooltip: []
  checkSingle: []
  toggle: []
  ytConfigToggle: [key: 'videos' | 'live', value: boolean]
  flagChange: [key: 'fetch_history' | 'check_prereleases', value: string]
  markAllRead: []
  muteToggle: []
  sendToAgent: []
  remove: []
}>()

function sourceTypeIcon(type: string): string {
  return getSourceTypeDef(type)?.icon ?? '/icons.svg#github-mark'
}

function sourceTypeTitle(type: string): string {
  const def = getSourceTypeDef(type)
  return def ? t(def.titleKey) : t('source.type_github')
}

function sourceHealthClass(source: Source): string {
  if (!source.enabled) return 'health-paused'
  if (source.last_check_status === 'ok') return 'health-ok'
  if (source.last_check_status === 'error') return 'health-error'
  return 'health-unknown'
}

function sourceHealthLabel(source: Source): string {
  if (!source.enabled) return t('source.health_paused')
  if (source.muted) return t('source.muted')
  if (source.last_check_status === 'ok') return t('source.no_pending_updates')
  if (source.last_check_status === 'error') return t('source.health_error')
  return t('source.health_unknown')
}

function sourceHealthAriaLabel(source: Source): string {
  if (!source.enabled) return t('source.health_paused')
  if (source.last_check_status === 'ok') return t('source.health_normal')
  if (source.last_check_status === 'error') return t('source.health_error')
  return t('source.health_unknown')
}

function sourceCheckedText(source: Source): string {
  if (!source.last_checked_at) return t('source.never_checked')
  return t('source.last_checked', formatDate(source.last_checked_at))
}
</script>

<template>
  <div
    class="source-item"
    :class="{ 'source-highlight': highlighted }"
    :draggable="agentEnabled && agentPanelOpen"
    @dragstart="emit('dragStart', $event)"
  >
    <div v-if="selectionMode" class="source-checkbox">
      <input
        type="checkbox"
        :checked="selected"
        @click="emit('checkboxClick', $event)"
        @change="emit('toggleSelect')"
      />
    </div>
    <div class="source-main">
      <div class="source-info">
        <span v-if="showSourceTypeIcons" class="source-type-badge" :class="source.source_type" :title="sourceTypeTitle(source.source_type)">
          <svg><use :href="sourceTypeIcon(source.source_type)" /></svg>
        </span>
        <span class="source-name">{{ sourceDisplayName(source) }}</span>
        <button
          class="btn-icon-link"
          :title="t('source.visit')"
          @click="emit('openUrl')"
          @contextmenu.prevent.stop="emit('contextMenuUrl', $event)"
        >
          <svg><use href="/icons.svg#link-icon" /></svg>
        </button>
        <button class="btn-icon-link" :title="t('source.view_releases')" @click="emit('openReleases')">
          <svg><use href="/icons.svg#search-icon" /></svg>
        </button>
        <span v-if="source.enabled && source.muted" class="source-status">{{ t('source.muted') }}</span>
        <span v-else-if="source.enabled" class="source-status source-status-on">{{ t('source.enabled') }}</span>
        <span v-else class="source-status source-status-off">{{ t('source.paused') }}</span>
      </div>
      <div class="source-health">
        <span
          class="health-dot"
          :class="sourceHealthClass(source)"
          :aria-label="sourceHealthAriaLabel(source)"
          @mouseenter="emit('showHealthTooltip', $event)"
          @mouseleave="emit('hideHealthTooltip')"
        ></span>
        <template v-if="source.enabled && source.last_check_status === 'ok'">
          <button
            v-if="unreadCount > 0"
            class="source-pending-link"
            @click="emit('openUnreadReleases')"
          >
            {{ t('source.pending_updates', String(unreadCount)) }}
          </button>
          <span class="source-health-meta">{{ sourceCheckedText(source) }}</span>
        </template>
        <template v-else>
          <span class="source-health-label">{{ sourceHealthLabel(source) }}</span>
          <span class="source-health-meta">{{ sourceCheckedText(source) }}</span>
          <span v-if="source.consecutive_failures > 0" class="source-health-meta">
            {{ t('source.failure_count', String(source.consecutive_failures)) }}
          </span>
        </template>
      </div>
      <div v-if="source.last_check_status === 'error' && source.last_check_message" class="source-error-block">
        <span class="source-error" :title="translateError(source.last_check_message)">
          {{ translateError(source.last_check_message) }}
        </span>
        <button type="button" class="btn-sm source-error-retry" :disabled="polling || checkingId !== null" @click="emit('checkSingle')">
          {{ t('common.retry') }}
        </button>
      </div>
    </div>
    <div class="source-actions">
      <button
        class="btn-icon-action btn-check"
        :disabled="polling || checkingId !== null"
        :title="checkingId === source.id ? t('source.checking') : t('source.check')"
        @click="emit('checkSingle')"
      >
        <svg><use href="/icons.svg#refresh-icon" /></svg>
      </button>
      <button
        class="btn-icon-action"
        :class="source.enabled ? 'btn-pause' : 'btn-resume'"
        :title="source.enabled ? t('source.pause') : t('source.resume')"
        @click="emit('toggle')"
      >
        <svg><use :href="source.enabled ? '/icons.svg#pause-icon' : '/icons.svg#play-icon'" /></svg>
      </button>
      <SourceCardMenu
        :source="source"
        :open="moreOpen"
        :unread-count="unreadCount"
        :agent-enabled="agentEnabled"
        :on-trigger-click="onMoreTriggerClick"
        :on-trigger-keydown="onMoreTriggerKeydown"
        :on-dropdown-keydown="onMoreDropdownKeydown"
        @yt-config-toggle="(k, v) => emit('ytConfigToggle', k, v)"
        @flag-change="(k, v) => emit('flagChange', k, v)"
        @mark-all-read="emit('markAllRead')"
        @mute-toggle="emit('muteToggle')"
        @send-to-agent="emit('sendToAgent')"
        @remove="emit('remove')"
      />
    </div>
  </div>
</template>

<style scoped>
.source-item {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 8px;
  background: transparent;
  border-bottom: 1px solid var(--border);
  border-radius: var(--radius-sm);
  transition: background var(--dur-hover) var(--ease-out);
}

.source-item:hover {
  background: var(--hover-surface);
}

.source-checkbox {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  padding: 0 4px;
}

.source-checkbox input[type="checkbox"] {
  width: 16px;
  height: 16px;
  cursor: pointer;
  accent-color: var(--primary);
}

.source-main {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.source-info {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.source-type-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  cursor: help;
}

.source-type-badge svg {
  width: 13px;
  height: 13px;
}

.source-type-badge.github {
  background: #181717;
  color: #ffffff;
}

.source-type-badge.huggingface {
  background: #ffffff;
  color: #ffd21e;
  border: 1px solid var(--border);
}

.source-type-badge.youtube {
  background: #ff0000;
  color: #ffffff;
}

.source-type-badge.bilibili {
  background: #fb7299;
  color: #ffffff;
}

.source-name {
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.source-health {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  font-size: var(--fs-meta);
  color: var(--text-muted);
}

.source-health-label {
  color: var(--text);
}

.source-pending-link {
  padding: 1px 7px;
  border: none;
  border-radius: var(--radius-xs);
  background: var(--primary-soft-bg);
  color: var(--primary-soft-text);
  font: inherit;
  font-size: var(--fs-meta);
  font-weight: 500;
  cursor: pointer;
  transition: background var(--dur-state) var(--ease-out);
}

.source-pending-link:hover {
  background: var(--primary-soft-border);
}

.source-pending-link:focus-visible {
  outline: 2px solid var(--primary-soft-border);
  outline-offset: 2px;
}

.source-health-meta {
  color: var(--text-muted);
}

.health-dot {
  position: relative;
  width: 7px;
  height: 7px;
  border-radius: var(--radius-pill);
  flex-shrink: 0;
  background: var(--text-muted);
}

.health-dot::before {
  content: '';
  position: absolute;
  top: -5px;
  left: -5px;
  right: -5px;
  bottom: -5px;
  border-radius: var(--radius-pill);
}

.health-ok {
  background: var(--success);
}

.health-error {
  background: var(--danger);
}

.health-paused,
.health-unknown {
  background: var(--text-muted);
}

.source-error {
  flex: 1;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-meta);
  color: var(--danger-soft-text);
}

.source-error-block {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  padding: 3px 4px 3px 8px;
  background: var(--danger-soft-bg);
  border-radius: var(--radius-xs);
}

.source-error-retry {
  flex-shrink: 0;
}

.source-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 0;
  background: transparent;
  font-size: var(--fs-meta);
  color: var(--text-muted);
}

.source-status::before {
  content: '';
  width: 6px;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--text-faint);
  flex-shrink: 0;
}

.source-status-on::before {
  background: var(--success);
}

.source-status-off::before {
  background: var(--warning);
}

.source-actions {
  display: flex;
  gap: 6px;
  align-items: center;
  justify-content: flex-end;
  flex-shrink: 0;
}

.source-highlight {
  animation: highlight-pulse 2s ease-out;
}

@keyframes highlight-pulse {
  0% {
    background-color: var(--primary-soft-bg);
  }
  100% {
    background-color: transparent;
  }
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

.btn-icon-action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-check:hover:not(:disabled),
.btn-pause:hover:not(:disabled),
.btn-resume:hover:not(:disabled) {
  background: var(--hover-surface-nested);
  color: var(--text);
}
</style>
