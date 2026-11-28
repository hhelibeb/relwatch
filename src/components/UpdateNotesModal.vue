<script setup lang="ts">
import { computed } from 'vue'
import BaseModal from './ui/BaseModal.vue'
import AppIcon from './ui/AppIcon.vue'
import MarkdownContent from './common/MarkdownContent.vue'
import EmptyState from './common/EmptyState.vue'
import { openReleaseUrl } from '../api/client'
import { track } from '../composables/useUsageTracking'
import { t } from '../i18n'
import { formatDate } from '../utils'

const props = defineProps<{
  version: string
  date: string | null
  body: string | null
}>()

const emit = defineEmits<{ close: [] }>()

const dateText = computed(() => (props.date ? formatDate(props.date) : ''))
const releaseUrl = computed(() => `https://github.com/hhelibeb/relwatch/releases/tag/v${props.version}`)

function handleClose() {
  track('update.notes_close')
  emit('close')
}

function handleOpenInBrowser(): void {
  track('update.notes_open_browser')
  void openReleaseUrl(releaseUrl.value)
}
</script>

<template>
  <BaseModal overlay-class="update-notes-overlay" modal-class="update-notes-modal" @close="handleClose">
    <div class="update-notes-header">
      <div class="update-notes-heading">
        <span class="update-notes-version">v{{ version }}</span>
        <span v-if="dateText" class="update-notes-date">{{ t('update.notes_built_at', dateText) }}</span>
      </div>
      <button class="update-notes-close" :title="t('release.detail_close')" @click="handleClose">
        <AppIcon name="close-icon" />
      </button>
    </div>
    <div class="update-notes-body">
      <div v-if="body" class="update-notes-markdown">
        <MarkdownContent :content="body" />
      </div>
      <EmptyState v-else class="update-notes-empty" :title="t('update.notes_empty')" compact />
    </div>
    <div class="update-notes-footer">
      <button class="btn-sm" @click="handleOpenInBrowser">{{ t('update.notes_open_in_browser') }}</button>
      <button class="btn-sm" @click="handleClose">{{ t('release.detail_close') }}</button>
    </div>
  </BaseModal>
</template>

<style scoped>
:global(.update-notes-modal) {
  width: min(620px, 100%);
  max-height: calc(100vh - 48px);
}

.update-notes-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 14px 16px 0;
}

.update-notes-heading {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.update-notes-version {
  font-weight: 600;
  font-size: var(--fs-title);
  color: var(--primary);
  flex-shrink: 0;
}

.update-notes-date {
  font-size: var(--fs-meta);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.update-notes-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: none;
  border-radius: var(--radius);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  flex-shrink: 0;
}

.update-notes-close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.update-notes-body {
  flex: 1;
  min-height: 120px;
  overflow-y: auto;
  padding: 10px 16px 14px;
}

.update-notes-empty {
  padding: 24px 12px;
}

.update-notes-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  padding: 10px 16px;
  border-top: 1px solid var(--border);
}
</style>
