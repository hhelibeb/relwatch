<script setup lang="ts">
import { ref, computed } from 'vue'
import { message } from '@tauri-apps/plugin-dialog'
import { type LogEntry, clearLogs } from '../api'
import { t } from '../i18n'
import { formatDate, logLevelClass } from '../utils'

const props = defineProps<{ logs: LogEntry[] }>()
const emit = defineEmits<{ update: [] }>()

const logSearch = ref('')

const filteredLogs = computed(() => {
  const q = logSearch.value.trim().toLowerCase()
  if (!q) return props.logs
  return props.logs.filter(l => l.message.toLowerCase().includes(q) || l.level.toLowerCase().includes(q))
})

async function handleClearLogs() {
  try {
    await clearLogs()
    emit('update')
  } catch (e: any) {
    await message(t('log.clear_failed') + (e?.toString?.() ?? String(e)), { title: t('settings.error'), kind: 'error' })
  }
}
</script>

<template>
  <section class="tab-content">
    <div class="log-search">
      <input
        v-model="logSearch"
        :placeholder="t('log.search')"
        class="search-input"
      />
      <button class="btn-icon" :title="t('log.clear')" @click="handleClearLogs">
        <svg><use href="/icons.svg#trash-icon"/></svg>
      </button>
    </div>
    <div class="log-list">
      <div v-if="filteredLogs.length === 0" class="empty">{{ logSearch ? t('log.no_match') : t('log.no_records') }}</div>
      <div v-for="entry in filteredLogs" :key="entry.id" class="log-item">
        <span class="log-level" :class="logLevelClass(entry.level)">{{ entry.level }}</span>
        <span class="log-msg">{{ entry.message }}</span>
        <span class="log-date">{{ formatDate(entry.created_at) }}</span>
      </div>
    </div>
  </section>
</template>
