<script setup lang="ts">
import { ref, computed } from 'vue'
import { message } from '@tauri-apps/plugin-dialog'
import { type LogEntry, clearLogs } from '../api'
import { t, tm } from '../i18n'
import { formatDate, logLevelClass } from '../utils'

const props = defineProps<{ logs: LogEntry[] }>()
const emit = defineEmits<{ update: [] }>()

const logSearch = ref('')

function renderMessage(entry: LogEntry): string {
  if (entry.message_key && entry.message_args) {
    try {
      const args = JSON.parse(entry.message_args)
      return tm(entry.message_key, args)
    } catch {
      return entry.message
    }
  }
  return entry.message
}

const filteredLogs = computed(() => {
  const q = logSearch.value.trim().toLowerCase()
  if (!q) return props.logs
  return props.logs.filter(l => {
    const text = renderMessage(l).toLowerCase()
    return text.includes(q) || l.level.toLowerCase().includes(q)
  })
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
        <span class="log-msg">{{ renderMessage(entry) }}</span>
        <span class="log-date">{{ formatDate(entry.created_at) }}</span>
      </div>
    </div>
  </section>
</template>
