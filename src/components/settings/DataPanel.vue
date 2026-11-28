<script setup lang="ts">
import { t } from '../../i18n'

defineProps<{
  githubTokenSet: boolean
}>()

const emit = defineEmits<{
  exportBackup: []
  importBackup: []
}>()
</script>

<template>
  <div class="settings-form data-panel">
    <h3 class="setting-section-title">{{ t('backup.section_title') }}</h3>
    <p class="setting-section-desc">
      {{ t('backup.section_desc') }}<template v-if="githubTokenSet"><br>{{ t('backup.token_note') }}</template>
    </p>
    <div class="setting-row backup-actions">
      <button class="btn-secondary" @click="emit('exportBackup')">{{ t('backup.export_btn') }}</button>
      <button class="btn-secondary btn-danger" @click="emit('importBackup')">{{ t('backup.import_btn') }}</button>
    </div>
  </div>
</template>

<style scoped>
.data-panel {
  gap: 13px;
}

.backup-actions {
  display: flex;
  flex-direction: row;
  gap: 10px;
  margin-top: 8px;
}
</style>
