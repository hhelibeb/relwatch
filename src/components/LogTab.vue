<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { type LogEntry, searchLogs, clearLogs } from '../api/logs'
import { t, tm } from '../i18n'
import { translateError } from '../api/client'
import { formatDate, logLevelClass } from '../utils'

const props = defineProps<{ refreshKey: number }>()
const emit = defineEmits<{ update: [] }>()

const logs = ref<LogEntry[]>([])
const totalLogs = ref(0)
const currentPage = ref(1)
const pageSize = 50
const loading = ref(false)
const searchKeyword = ref('')
const levelFilter = ref('all')
const pageInput = ref('')
const debounceTimer = ref<ReturnType<typeof setTimeout> | null>(null)

// 级别过滤下拉
const openFilter = ref(false)
let hoverFilterTimer: ReturnType<typeof setTimeout> | null = null

function hoverFilterEnter() {
  if (hoverFilterTimer) {
    clearTimeout(hoverFilterTimer)
    hoverFilterTimer = null
  }
}

function hoverFilterLeave() {
  hoverFilterTimer = setTimeout(() => {
    openFilter.value = false
  }, 120)
}

const totalPages = computed(() => Math.max(1, Math.ceil(totalLogs.value / pageSize)))

const pageInputStyle = computed(() => ({
  width: `${String(totalPages.value).length + 3}ch`
}))

async function loadData() {
  loading.value = true
  try {
    const result = await searchLogs(searchKeyword.value, currentPage.value, pageSize, levelFilter.value === 'all' ? undefined : levelFilter.value)
    logs.value = result.entries
    totalLogs.value = result.total
    pageInput.value = String(currentPage.value)
  } finally {
    loading.value = false
  }
}

function goPage(page: number) {
  if (page < 1 || page > totalPages.value || loading.value) return
  currentPage.value = page
  pageInput.value = String(page)
  loadData()
}

function jumpToPage() {
  const page = parseInt(pageInput.value, 10)
  if (isNaN(page) || page < 1) {
    pageInput.value = String(currentPage.value)
    return
  }
  goPage(Math.min(page, totalPages.value))
}

function onSearchInput() {
  if (debounceTimer.value) clearTimeout(debounceTimer.value)
  debounceTimer.value = setTimeout(() => {
    currentPage.value = 1
    loadData()
  }, 300)
}

function clearSearch() {
  searchKeyword.value = ''
  currentPage.value = 1
  loadData()
}

function setLevelFilter(level: string) {
  levelFilter.value = level
  openFilter.value = false
  currentPage.value = 1
  loadData()
}

function renderMessage(entry: LogEntry): string {
  if (entry.message_key && entry.message_args) {
    try {
      const args: Record<string, string> = JSON.parse(entry.message_args)
      // 翻译 Rust 后端传入的 err.* 格式错误文本
      if (args.error && args.error.startsWith('err.')) {
        args.error = translateError(args.error)
      }
      return tm(entry.message_key, args)
    } catch {
      return entry.message
    }
  }
  return entry.message
}

async function handleClearLogs() {
  const confirmed = await confirm(t('log.clear_confirm'), { title: t('log.clear'), kind: 'warning' })
  if (!confirmed) return
  try {
    await clearLogs()
    currentPage.value = 1
    await loadData()
    emit('update')
  } catch (e: unknown) {
    await message(t('log.clear_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
}

watch(() => props.refreshKey, () => {
  loadData()
})

onMounted(() => {
  loadData()
})
</script>

<template>
  <section class="tab-content">
    <div class="log-search-row">
      <div class="input-clear-wrap">
        <input
          v-model="searchKeyword"
          :placeholder="t('log.search')"
          class="search-input"
          @input="onSearchInput"
        />
        <button v-if="searchKeyword" type="button" class="input-clear-btn" :title="t('input.clear')" @click="clearSearch">✕</button>
      </div>
      <div class="filter-group" @mouseleave="hoverFilterLeave()">
        <div class="filter-field" @mouseenter="openFilter = true; hoverFilterEnter()">
          <button class="filter-trigger">
            <span class="filter-label">{{ t('log.level') }}</span>
            <span class="filter-value" :style="{ color: levelFilter === 'all' ? 'var(--text-muted)' : levelFilter === 'ERROR' ? 'var(--danger)' : levelFilter === 'WARN' ? 'var(--warning)' : 'var(--text-muted)' }">{{ levelFilter === 'all' ? t('log.filter_all') : levelFilter }}</span>
            <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
          </button>
          <div v-if="openFilter" class="filter-dropdown" @mouseenter="hoverFilterEnter()" @mouseleave="hoverFilterLeave()">
            <button :class="{ selected: levelFilter === 'all' }" @click="setLevelFilter('all')">{{ t('log.filter_all') }}</button>
            <button :class="{ selected: levelFilter === 'INFO' }" @click="setLevelFilter('INFO')" style="color:var(--text-muted)">INFO</button>
            <button :class="{ selected: levelFilter === 'WARN' }" @click="setLevelFilter('WARN')" style="color:var(--warning)">WARN</button>
            <button :class="{ selected: levelFilter === 'ERROR' }" @click="setLevelFilter('ERROR')" style="color:var(--danger)">ERROR</button>
          </div>
        </div>
      </div>
      <button class="btn-icon" :title="t('log.clear')" @click="handleClearLogs">
        <svg><use href="/icons.svg#trash-icon"/></svg>
      </button>
    </div>
    <div class="log-list">
      <div v-if="loading" class="empty">{{ t('log.loading') }}</div>
      <div v-else-if="logs.length === 0" class="empty">{{ searchKeyword ? t('log.no_match') : t('log.no_records') }}</div>
      <div v-for="entry in logs" :key="entry.id" class="log-item">
        <span class="log-level" :class="logLevelClass(entry.level)">{{ entry.level }}</span>
        <span class="log-msg">{{ renderMessage(entry) }}</span>
        <span class="log-date">{{ formatDate(entry.created_at) }}</span>
      </div>
    </div>
    <div v-if="totalPages > 1" class="pagination">
      <button class="btn-sm pagination-btn" :disabled="currentPage <= 1 || loading" @click="goPage(currentPage - 1)">{{ t('log.prev_page') }}</button>
      <div class="pagination-page-group">
        <input class="pagination-input" v-model="pageInput" type="number" min="1" :max="totalPages" :style="pageInputStyle" @keyup.enter="jumpToPage" @blur="jumpToPage" />
        <span class="pagination-info">/ {{ totalPages }}</span>
      </div>
      <button class="btn-sm pagination-btn" :disabled="currentPage >= totalPages || loading" @click="goPage(currentPage + 1)">{{ t('log.next_page') }}</button>
      <span class="pagination-total">{{ t('log.total_entries', String(totalLogs)) }}</span>
    </div>
  </section>
</template>
