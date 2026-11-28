<script setup lang="ts">
import { computed } from 'vue'
import { version } from '../../../package.json'
import type { useAppUpdate } from '../../composables/useAppUpdate'
import { t, tm } from '../../i18n'
import UpdateNotesModal from '../UpdateNotesModal.vue'

const props = defineProps<{
  update: ReturnType<typeof useAppUpdate>
}>()

const {
  status: updateStatus,
  currentVersion: updateCurrentVersion,
  pendingUpdate,
  errorKind: updateErrorKind,
  errorText: updateErrorText,
  percent: updatePercent,
  downloadText: updateDownloadText,
  busy: updateBusy,
  showNotes: showUpdateNotes,
  notesVersion: updateNotesVersion,
  notesDate: updateNotesDate,
  notesBody: updateNotesBody,
  checkForUpdate,
  downloadAndInstall,
  retry: retryUpdate,
  openReleaseNotes: openUpdateNotes,
  openDownloadPage,
} = props.update

const isDevBuild = import.meta.env.DEV
const showUpdateRetry = computed(
  () => updateErrorKind.value === 'network' || updateErrorKind.value === 'generic' || updateErrorKind.value === 'no_release',
)
const showUpdateDownloadPage = computed(() => updateErrorKind.value !== 'unsupported')
</script>

<template>
  <div class="settings-form">
    <div class="about-header">
      <div class="about-app-name">{{ t('about.app_name') }}</div>
      <div class="about-version">{{ t('about.version') }} v{{ version }}</div>
      <p class="setting-section-desc">{{ t('about.app_desc') }}</p>
      <div class="setting-row">
        <a class="setting-link" href="https://github.com/hhelibeb/relwatch">{{ t('about.github') }}: github.com/hhelibeb/relwatch</a>
      </div>
    </div>
    <hr class="setting-divider" />
    <div class="setting-section-title">{{ t('update.section_title') }}</div>
    <div class="setting-row setting-row-checkbox">
      <span class="setting-label">{{ t('update.current_version') }} <strong>v{{ updateCurrentVersion }}</strong></span>
      <button
        class="btn-secondary"
        :disabled="isDevBuild || updateBusy"
        :title="isDevBuild ? t('update.dev_disabled') : undefined"
        data-testid="update-check-btn"
        @click="checkForUpdate"
      >
        {{ updateStatus === 'checking' ? t('update.checking') : t('update.check') }}
      </button>
      <span v-if="isDevBuild" class="setting-note">{{ t('update.dev_disabled') }}</span>
    </div>
    <div v-if="updateStatus === 'upToDate'" class="setting-row">
      <span class="update-status-ok">✓ {{ tm('update.up_to_date', { version: `v${updateCurrentVersion}` }) }}</span>
    </div>
    <div v-else-if="updateStatus === 'available'" class="setting-row">
      <span class="setting-label">{{ tm('update.available', { version: `v${pendingUpdate?.version ?? ''}` }) }}</span>
      <div class="update-actions">
        <button class="btn-secondary" :disabled="updateBusy" data-testid="update-install-btn" @click="downloadAndInstall">{{ t('update.download_install') }}</button>
        <button class="btn-secondary" :disabled="updateBusy" data-testid="update-notes-btn" @click="openUpdateNotes">{{ t('update.view_notes') }}</button>
      </div>
    </div>
    <div v-else-if="updateStatus === 'downloading'" class="setting-row">
      <div class="update-progress">
        <div class="update-progress-inner" :style="{ width: `${updatePercent ?? 0}%` }"></div>
      </div>
      <span class="setting-note">{{ updateDownloadText }}</span>
    </div>
    <div v-else-if="updateStatus === 'installing'" class="setting-row">
      <span class="setting-label">{{ t('update.installing') }}</span>
    </div>
    <div v-else-if="updateStatus === 'error'" class="setting-row">
      <span class="update-status-error">⚠ {{ updateErrorText }}</span>
      <div v-if="showUpdateRetry || showUpdateDownloadPage" class="update-actions">
        <button v-if="showUpdateRetry" class="btn-secondary" @click="retryUpdate">{{ t('update.retry') }}</button>
        <button v-if="showUpdateDownloadPage" class="btn-secondary" @click="openDownloadPage">{{ t('update.open_download_page') }}</button>
      </div>
    </div>

    <UpdateNotesModal
      v-if="showUpdateNotes && updateNotesBody"
      :version="updateNotesVersion"
      :date="updateNotesDate"
      :body="updateNotesBody"
      @close="showUpdateNotes = false"
    />
  </div>
</template>

<style scoped>
.about-header {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 4px 0 8px;
}

.about-app-name {
  font-size: var(--fs-display);
  font-weight: 700;
}

.about-version {
  font-size: var(--fs-body);
  color: var(--text-muted);
}

.update-actions {
  display: flex;
  gap: 10px;
  margin-top: 4px;
}

.update-progress {
  width: 100%;
  max-width: 320px;
  height: 6px;
  border-radius: var(--radius-xs);
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  overflow: hidden;
}

.update-progress-inner {
  height: 100%;
  background: var(--primary);
  transition: width var(--dur-overlay) var(--ease-out);
}

.update-status-ok {
  font-size: var(--fs-body);
  color: var(--text);
}

.update-status-error {
  font-size: var(--fs-body);
  color: var(--warning);
  line-height: 1.5;
}
</style>
