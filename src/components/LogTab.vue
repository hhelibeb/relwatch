<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { type LogEntry, searchLogs, clearLogs } from '../api/logs'
import { t, tm } from '../i18n'
import { translateError } from '../api/client'
import { formatDate, logLevelClass } from '../utils'
import { useDropdown } from '../composables/useDropdown'
import { track } from '../composables/useUsageTracking'

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
const levelDropdown = useDropdown({
  openState: openFilter,
  closedKey: false,
  hoverOpen: true,
  // 打开时聚焦第一个选项；从触发元素就近定位
  onOpen: (_key, el) => {
    const dropdown = el.parentElement?.querySelector('.filter-dropdown') as HTMLElement | null
    dropdown?.querySelector('button')?.focus()
  },
})

// 自增标记：用于丢弃并发场景下陈旧响应，避免旧请求结果覆盖新数据
let loadId = 0

const totalPages = computed(() => Math.max(1, Math.ceil(totalLogs.value / pageSize)))

const pageInputStyle = computed(() => ({
  width: `${String(totalPages.value).length + 3}ch`
}))

async function loadData() {
  const id = ++loadId
  loading.value = true
  try {
    const result = await searchLogs(searchKeyword.value, currentPage.value, pageSize, levelFilter.value === 'all' ? undefined : levelFilter.value)
    // 并发场景下若已有更新的调用发起，丢弃本次陈旧响应
    if (id !== loadId) return
    logs.value = result.entries
    totalLogs.value = result.total
    pageInput.value = String(currentPage.value)
  } catch {
    // 查询失败时清空列表（仅最新调用负责），交给空状态 UI 呈现，避免未捕获 rejection
    if (id === loadId) {
      logs.value = []
      totalLogs.value = 0
    }
  } finally {
    // 仅最新调用负责复位 loading，避免被陈旧响应提前清除
    if (id === loadId) loading.value = false
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
  track('log.filter')
  levelFilter.value = level
  levelDropdown.close()
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
  track('log.clear')
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

onUnmounted(() => {
  // 清理未触发的定时器，避免卸载后回调修改已销毁组件的 ref
  if (debounceTimer.value) {
    clearTimeout(debounceTimer.value)
    debounceTimer.value = null
  }
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
      <div class="filter-group" @mouseleave="levelDropdown.hoverLeave()">
        <div class="filter-field" @mouseenter="levelDropdown.hoverEnter(true)">
          <button type="button" class="filter-trigger" :aria-expanded="openFilter" aria-haspopup="menu" @click="levelDropdown.toggle($event, true)" @keydown="levelDropdown.handleTriggerKeydown($event, true)">
            <span class="filter-label">{{ t('log.level') }}</span>
            <span class="filter-value" :style="{ color: levelFilter === 'all' ? 'var(--text-muted)' : levelFilter === 'ERROR' ? 'var(--danger)' : levelFilter === 'WARN' ? 'var(--warning)' : 'var(--text-muted)' }">{{ levelFilter === 'all' ? t('log.filter_all') : levelFilter }}</span>
            <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
          </button>
          <div v-if="openFilter" class="filter-dropdown" role="menu" @mouseenter="levelDropdown.hoverEnter(true)" @mouseleave="levelDropdown.hoverLeave()" @keydown="levelDropdown.handleDropdownKeydown">
            <button type="button" role="menuitem" :aria-selected="levelFilter === 'all'" :class="{ selected: levelFilter === 'all' }" @click="setLevelFilter('all')">{{ t('log.filter_all') }}</button>
            <button type="button" role="menuitem" :aria-selected="levelFilter === 'INFO'" :class="{ selected: levelFilter === 'INFO' }" @click="setLevelFilter('INFO')" style="color:var(--text-muted)">INFO</button>
            <button type="button" role="menuitem" :aria-selected="levelFilter === 'WARN'" :class="{ selected: levelFilter === 'WARN' }" @click="setLevelFilter('WARN')" style="color:var(--warning)">WARN</button>
            <button type="button" role="menuitem" :aria-selected="levelFilter === 'ERROR'" :class="{ selected: levelFilter === 'ERROR' }" @click="setLevelFilter('ERROR')" style="color:var(--danger)">ERROR</button>
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
<style scoped>
.log-search {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 12px;
  padding: 0;
  background: var(--bg);
  transition: top 0.15s ease;
}

.btn-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
  flex-shrink: 0;
}

.btn-icon svg {
  width: 16px;
  height: 16px;
  color: var(--text-muted);
}

.btn-icon:hover {
  background: var(--danger-soft-bg);
  border-color: var(--danger);
}

.btn-icon:hover svg {
  color: var(--danger);
}

.log-list {
  display: flex;
  flex-direction: column;
}

.log-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 7px 8px;
  background: transparent;
  border-bottom: 1px solid var(--border);
  font-size: 13px;
}

.log-level {
  font-weight: 600;
  flex-shrink: 0;
  min-width: 42px;
}

.log-error { color: var(--danger); }
.log-warn { color: var(--warning); }
.log-info { color: var(--text-muted); }

.log-msg {
  flex: 1;
  word-break: break-all;
}

.log-date {
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: 12px;
  white-space: nowrap;
}

.pagination {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 16px 0 4px;
  flex-wrap: wrap;
}

.pagination-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.pagination-page-group {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.pagination-input {
  height: 26px;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text);
  font-size: 12px;
  line-height: 24px;
  text-align: center;
  outline: none;
  transition: border-color 0.2s;
}

.pagination-input:focus {
  border-color: var(--primary);
}

.pagination-input::-webkit-outer-spin-button,
.pagination-input::-webkit-inner-spin-button {
  -webkit-appearance: none;
  margin: 0;
}

.pagination-input[type="number"] {
  -moz-appearance: textfield;
}

.pagination-info {
  font-size: 12px;
  color: var(--text);
  white-space: nowrap;
  line-height: 26px;
}

.pagination-total {
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}

</style>
