<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { type LogEntry, searchLogs, clearLogs } from '../api/logs'
import { t, tm } from '../i18n'
import { translateError } from '../api/client'
import { formatDate, logLevelClass } from '../utils'
import { useDropdown } from '../composables/useDropdown'
import { track } from '../composables/useUsageTracking'
import EmptyState from './common/EmptyState.vue'
import ErrorBlock from './common/ErrorBlock.vue'
import SkeletonList from './common/SkeletonList.vue'

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
/* 查询失败不再只清空列表：空列表配「暂无日志」会把拉取失败说成没有数据 */
const error = ref('')
const debounceTimer = ref<ReturnType<typeof setTimeout> | null>(null)

const openFilter = ref(false)
const levelDropdown = useDropdown({
  openState: openFilter,
  closedKey: false,
  hoverOpen: true,
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
    if (id !== loadId) return
    logs.value = result.entries
    totalLogs.value = result.total
    pageInput.value = String(currentPage.value)
    error.value = ''
  } catch (e: unknown) {
    // 查询失败时清空列表（仅最新调用负责），失败原因留在原位供重试
    if (id === loadId) {
      logs.value = []
      totalLogs.value = 0
      error.value = t('app.load_failed', e instanceof Error ? e.message : String(e))
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

// 空态出口：搜索与级别筛选一起清掉（只清一项时结果仍可能为空，用户会以为兜底没生效）
const hasLogSearch = computed(() => searchKeyword.value.trim() !== '')
const hasLogFilter = computed(() => hasLogSearch.value || levelFilter.value !== 'all')

function clearLogFilters() {
  searchKeyword.value = ''
  levelFilter.value = 'all'
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
          <div v-if="openFilter" class="dropdown-panel filter-dropdown" role="menu" @mouseenter="levelDropdown.hoverEnter(true)" @mouseleave="levelDropdown.hoverLeave()" @keydown="levelDropdown.handleDropdownKeydown">
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
      <SkeletonList v-if="loading && logs.length === 0" :rows="3" :label="t('common.loading')" />
      <ErrorBlock v-else-if="error" :message="error" :retry-label="t('common.retry')" @retry="loadData" />
      <EmptyState
        v-else-if="logs.length === 0"
        :title="hasLogFilter ? t('log.no_match') : t('log.no_records')"
        :description="hasLogFilter ? t('log.no_match_desc') : ''"
      >
        <template v-if="hasLogFilter" #actions>
          <button type="button" class="btn-sm" @click="clearLogFilters">
            {{ hasLogSearch ? t('common.clear_search') : t('release.filter_clear_all') }}
          </button>
        </template>
      </EmptyState>
      <div v-for="entry in logs" :key="entry.id" class="log-item" :class="logLevelClass(entry.level)">
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
  z-index: var(--z-sticky);
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 12px;
  padding: 0;
  background: var(--bg);
  transition: top var(--dur-state) var(--ease-out);
}

.btn-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  cursor: pointer;
  transition: background var(--dur-state) var(--ease-out), border-color var(--dur-state) var(--ease-out);
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
  /* 左侧 2px 留给级别色条，padding-left 相应减 2，文本左边界与原来对齐 */
  padding: 7px 8px 7px 6px;
  background: transparent;
  border-left: 2px solid transparent;
  border-bottom: 1px solid var(--border);
  font-size: var(--fs-body);
  transition: background-color var(--dur-hover) var(--ease-out);
}

.log-item:hover {
  background: var(--hover-surface);
}

/* 级别由左侧色条编码，文字统一退回 muted：
   红字配红条是同一信息两处编码，一行里只会更吵。 */
.log-item.log-error { border-left-color: var(--danger); }
.log-item.log-warn { border-left-color: var(--warning); }

.log-level {
  font-weight: 600;
  flex-shrink: 0;
  min-width: 42px;
  color: var(--text-muted);
}

.log-msg {
  flex: 1;
  word-break: break-all;
}

.log-date {
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: var(--fs-meta);
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
  border-radius: var(--radius);
  background: var(--input-bg);
  color: var(--text);
  font-size: var(--fs-meta);
  line-height: 24px;
  text-align: center;
  outline: none;
  transition: border-color var(--dur-overlay) var(--ease-out);
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
  font-size: var(--fs-meta);
  color: var(--text);
  white-space: nowrap;
  line-height: 26px;
}

.pagination-total {
  font-size: var(--fs-meta);
  color: var(--text-muted);
  white-space: nowrap;
}

</style>
