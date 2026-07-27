<script setup lang="ts">
import { computed, inject, ref, nextTick, onMounted, onUnmounted, watch } from 'vue'
import { ShowToastKey } from '../injection-keys'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { type Source, parseSourceUrl, addSource, removeSource, updateSource } from '../api/sources'
import { checkSingleSource } from '../api/releases'
import { openReleaseUrl, translateError } from '../api/client'
import { useContextMenu } from '../composables/useContextMenu'
import ContextMenu from './common/ContextMenu.vue'
import { t, tm } from '../i18n'
import { formatDate } from '../utils'

const props = defineProps<{ sources: Source[]; polling: boolean; unreadReleaseCounts: Record<string, number>; totalReleaseCounts: Record<string, number>; showSourceTypeIcons: boolean }>()
const emit = defineEmits<{
  update: []
  checkResult: [count: number]
  checkBusy: [busy: boolean]
  openReleases: [query: string]
  openUnreadReleases: [query: string]
}>()

const urlInput = ref('')
const loading = ref(false)
const checkingId = ref<number | null>(null)
const highlightedId = ref<number | null>(null)
const openMoreId = ref<number | null>(null)
const showToast = inject(ShowToastKey, () => {})
// 新增源高亮动画的定时器句柄；连续添加时先 clear 旧定时器，避免旧回调截断新源高亮
let highlightTimer: ReturnType<typeof setTimeout> | undefined

const sourceSearch = ref('')
const trimmedSourceSearch = computed(() => sourceSearch.value.trim())
const hasActiveSourceSearch = computed(() => trimmedSourceSearch.value.length > 0)

const SOURCE_SORT_FIELD_STORAGE_KEY = 'relwatch.source.sort.field'
const SOURCE_SORT_DIRECTION_STORAGE_KEY = 'relwatch.source.sort.direction'

type SourceSortField = 'default' | 'name' | 'status' | 'created'
type SortDirection = 'asc' | 'desc'

function isSourceSortField(value: string | null): value is SourceSortField {
  return value === 'default' || value === 'name' || value === 'status' || value === 'created'
}

function isSortDirection(value: string | null): value is SortDirection {
  return value === 'asc' || value === 'desc'
}

function readStoredSourceSortField(): SourceSortField {
  try {
    const value = window.localStorage.getItem(SOURCE_SORT_FIELD_STORAGE_KEY)
    return isSourceSortField(value) ? value : 'default'
  } catch {
    return 'default'
  }
}

function readStoredSourceSortDirection(): SortDirection {
  try {
    const value = window.localStorage.getItem(SOURCE_SORT_DIRECTION_STORAGE_KEY)
    return isSortDirection(value) ? value : 'desc'
  } catch {
    return 'desc'
  }
}

type HeaderMode = 'add' | 'search'
const sourceSortField = ref<SourceSortField>(readStoredSourceSortField())
const sourceSortDirection = ref<SortDirection>(readStoredSourceSortDirection())
const openSort = ref(false)
const headerMode = ref<HeaderMode>('add')

const selectionMode = ref(false)
const selectedSourceIds = ref<Set<number>>(new Set())
const bulkBusy = ref(false)

function toggleSelection(sourceId: number) {
  const next = new Set(selectedSourceIds.value)
  if (next.has(sourceId)) next.delete(sourceId)
  else next.add(sourceId)
  selectedSourceIds.value = next
}

function selectAllVisible() {
  selectedSourceIds.value = new Set(sortedSources.value.map(s => s.id))
}

function clearSelectedSources() {
  selectedSourceIds.value = new Set()
}

function clearSelection() {
  selectedSourceIds.value = new Set()
  selectionMode.value = false
}

const selectedCount = computed(() => selectedSourceIds.value.size)

async function handleBulkToggle(enabled: boolean) {
  const ids = [...selectedSourceIds.value]
  if (ids.length === 0) return
  bulkBusy.value = true
  let success = 0, failed = 0
  for (const id of ids) {
    try {
      const source = props.sources.find(s => s.id === id)
      if (!source) continue
      await updateSource(id, enabled, source.poll_interval_minutes)
      success++
    } catch { failed++ }
  }
  bulkBusy.value = false
  if (failed > 0) showToast?.(t('source.bulk_result', String(success), String(failed)))
  emit('update')
}

async function handleBulkMuteToggle(muted: boolean) {
  const ids = [...selectedSourceIds.value]
  if (ids.length === 0) return
  bulkBusy.value = true
  let success = 0, failed = 0
  for (const id of ids) {
    try {
      const source = props.sources.find(s => s.id === id)
      // 跳过已暂停源：对暂停源执行静音无意义，与单源静音按钮 disabled 语义一致
      if (!source || !source.enabled) continue
      await updateSource(id, source.enabled, source.poll_interval_minutes, muted)
      success++
    } catch { failed++ }
  }
  bulkBusy.value = false
  if (failed > 0) showToast?.(t('source.bulk_result', String(success), String(failed)))
  emit('update')
}

async function handleBulkRemove() {
  const ids = [...selectedSourceIds.value]
  if (ids.length === 0) return
  const confirmed = await confirm(t('source.bulk_delete_confirm', String(ids.length)), { title: t('source.delete'), kind: 'warning' })
  if (!confirmed) return
  bulkBusy.value = true
  let success = 0, failed = 0
  for (const id of ids) {
    try {
      await removeSource(id)
      success++
    } catch { failed++ }
  }
  bulkBusy.value = false
  if (failed > 0) showToast?.(t('source.bulk_result', String(success), String(failed)))
  clearSelectedSources()
  emit('update')
}

function sourceMatchesSearch(source: Source, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  const name = `${source.owner}/${source.repo}`.toLowerCase()
  return name.includes(q) ||
    source.owner.toLowerCase().includes(q) ||
    source.repo.toLowerCase().includes(q) ||
    (source.description ?? '').toLowerCase().includes(q)
}

const filteredSources = computed(() => {
  const q = trimmedSourceSearch.value
  if (!q) return props.sources
  return props.sources.filter(s => sourceMatchesSearch(s, q))
})

const sortFieldOptions = computed(() => [
  { value: 'default' as const, label: t('source.sort_default') },
  { value: 'name' as const, label: t('source.sort_name') },
  { value: 'status' as const, label: t('source.sort_status') },
  { value: 'created' as const, label: t('source.sort_created') },
])

const sortDirectionOptions = computed(() => [
  { value: 'asc' as const, label: t('source.sort_asc') },
  { value: 'desc' as const, label: t('source.sort_desc') },
])

const { contextMenu, closeContextMenu, handleContextMenu, handleCopyLink, handleOpenLink } = useContextMenu()

function openSourceUrl(source: Source) {
  if (source.source_type === 'huggingface') {
    openReleaseUrl(`https://huggingface.co/${source.owner}`)
  } else {
    openReleaseUrl(`https://github.com/${source.owner}/${source.repo}`)
  }
}

function sourceQuery(source: Source): string {
  return `${source.owner}/${source.repo}`
}

function sourceKey(source: Source): string {
  return sourceQuery(source).toLowerCase()
}

function unreadReleaseCount(source: Source): number {
  return props.unreadReleaseCounts[sourceKey(source)] || 0
}

function openSourceReleases(source: Source) {
  emit('openReleases', sourceQuery(source))
}

function openSourceUnreadReleases(source: Source) {
  emit('openUnreadReleases', sourceQuery(source))
}

function sourceExists(owner: string, repo: string): boolean {
  return props.sources.some(source =>
    source.owner.toLowerCase() === owner.toLowerCase() &&
    source.repo.toLowerCase() === repo.toLowerCase()
  )
}

async function handleAdd() {
  const raw = urlInput.value.trim()
  if (!raw) return
  const parsed = parseSourceUrl(raw)
  if (!parsed) {
    await message(t('source.invalid_url'), { title: t('source.input_invalid'), kind: 'warning' })
    return
  }
  if (sourceExists(parsed.owner, parsed.repo)) {
    showToast?.(t('source.exists'))
    return
  }
  loading.value = true
  try {
    const id = await addSource(parsed.type, parsed.owner, parsed.repo)
    if (id === 0) {
      showToast?.(t('source.exists'))
      return
    }
    urlInput.value = ''
    highlightedId.value = id
    clearTimeout(highlightTimer)
    highlightTimer = setTimeout(() => { highlightedId.value = null }, 2200)
    emit('update')
  } catch (e: unknown) {
    const errMsg = e instanceof Error ? e.message : String(e)
    await message(tm('source.add_failed', { source_type: parsed.type, owner: parsed.owner, repo: parsed.repo, error: errMsg }), { title: t('settings.error'), kind: 'error' })
    emit('update')
  } finally {
    loading.value = false
  }
}

async function handleRemove(id: number) {
  try {
    await removeSource(id)
    emit('update')
  } catch (e: unknown) {
    await message(t('source.delete_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
}

async function handleToggle(source: Source) {
  try {
    await updateSource(source.id, !source.enabled, source.poll_interval_minutes)
    emit('update')
  } catch (e: unknown) {
    await message(t('source.operation_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
  openMoreId.value = null
}

async function handleMuteToggle(source: Source) {
  if (!source.enabled) return
  try {
    await updateSource(source.id, source.enabled, source.poll_interval_minutes, !source.muted)
    emit('update')
  } catch (e: unknown) {
    await message(t('source.operation_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
  openMoreId.value = null
}

function toggleMore(id: number) {
  openMoreId.value = openMoreId.value === id ? null : id
  if (openMoreId.value === id) {
    focusMorePanel()
  }
}

function handleSortTriggerKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (openSort.value) {
      openSort.value = false
    }
    return
  }
  if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
    e.preventDefault()
    if (!openSort.value) {
      openSort.value = true
      focusSortDropdown()
    }
  }
}

function handleSortDropdownKeydown(e: KeyboardEvent) {
  const target = e.target as HTMLElement
  if (!target || target.tagName !== 'BUTTON') return
  const dropdown = target.closest('.sort-dropdown') as HTMLElement | null
  if (!dropdown) return
  const buttons = Array.from(dropdown.querySelectorAll('button')) as HTMLButtonElement[]
  const index = buttons.indexOf(target as HTMLButtonElement)
  if (index < 0) return
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    const next = (index + 1) % buttons.length
    buttons[next].focus()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    const prev = (index - 1 + buttons.length) % buttons.length
    buttons[prev].focus()
  } else if (e.key === 'Escape') {
    e.preventDefault()
    openSort.value = false
  }
}

function focusSortDropdown() {
  requestAnimationFrame(() => {
    const dropdown = document.querySelector('.sort-dropdown') as HTMLElement | null
    if (!dropdown) return
    const btn = dropdown.querySelector('button') as HTMLButtonElement | null
    if (btn) btn.focus()
  })
}

function handleMoreKeydown(e: KeyboardEvent, sourceId: number) {
  if (e.key === 'Escape') {
    if (openMoreId.value === sourceId) {
      openMoreId.value = null
    }
    return
  }
  if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
    e.preventDefault()
    if (openMoreId.value !== sourceId) {
      openMoreId.value = sourceId
      focusMorePanel()
    }
  }
}

function handleMorePanelKeydown(e: KeyboardEvent) {
  const target = e.target as HTMLElement
  if (!target || target.tagName !== 'BUTTON') return
  const panel = target.closest('.dropdown-more-panel') as HTMLElement | null
  if (!panel) return
  const buttons = Array.from(panel.querySelectorAll('button')) as HTMLButtonElement[]
  const index = buttons.indexOf(target as HTMLButtonElement)
  if (index < 0) return
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    const next = (index + 1) % buttons.length
    buttons[next].focus()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    const prev = (index - 1 + buttons.length) % buttons.length
    buttons[prev].focus()
  } else if (e.key === 'Escape') {
    e.preventDefault()
    openMoreId.value = null
  }
}

function focusMorePanel() {
  requestAnimationFrame(() => {
    const sourceItems = document.querySelectorAll('.source-item')
    for (const item of sourceItems) {
      const panel = item.querySelector(`.dropdown-more-panel`) as HTMLElement | null
      if (panel) {
        const btn = panel.querySelector('button:not(:disabled)') as HTMLButtonElement | null
        if (btn) {
          btn.focus()
          return
        }
      }
    }
  })
}

function onDocumentClick() {
  openMoreId.value = null
  openSort.value = false
}

onMounted(() => document.addEventListener('click', onDocumentClick))
onUnmounted(() => document.removeEventListener('click', onDocumentClick))

watch(sourceSortField, value => {
  try {
    window.localStorage.setItem(SOURCE_SORT_FIELD_STORAGE_KEY, value)
  } catch { /* ignore unavailable storage */ }
})

watch(sourceSortDirection, value => {
  try {
    window.localStorage.setItem(SOURCE_SORT_DIRECTION_STORAGE_KEY, value)
  } catch { /* ignore unavailable storage */ }
})

async function handleCheckSingle(id: number) {
  if (props.polling || checkingId.value !== null) return
  checkingId.value = id
  emit('checkBusy', true)
  try {
    const result = await checkSingleSource(id)
    emit('update')
    emit('checkResult', result.new_releases.length)
  } catch (e: unknown) {
    await message(t('source.check_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  } finally {
    checkingId.value = null
    emit('checkBusy', false)
  }
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
  if (source.last_check_status === 'ok') {
    return t('source.no_pending_updates')
  }
  if (source.last_check_status === 'error') return t('source.health_error')
  return t('source.health_unknown')
}

const sortedSources = computed(() => {
  const direction = sourceSortDirection.value === 'asc' ? 1 : -1
  return [...filteredSources.value].sort((a, b) => {
    let result: number
    if (sourceSortField.value === 'default') {
      const aPending = props.unreadReleaseCounts[sourceKey(a)] || 0
      const bPending = props.unreadReleaseCounts[sourceKey(b)] || 0
      result = aPending - bPending || a.id - b.id
    } else if (sourceSortField.value === 'name') {
      result = `${a.owner}/${a.repo}`.localeCompare(`${b.owner}/${b.repo}`)
    } else if (sourceSortField.value === 'status') {
      if (a.enabled !== b.enabled) result = a.enabled ? -1 : 1
      else result = b.id - a.id
    } else {
      result = a.created_at.localeCompare(b.created_at) || a.id - b.id
    }
    return result * direction
  })
})

const sortLabelText = computed(() => {
  const labels: Record<SourceSortField, string> = {
    'default': t('source.sort_default'),
    'name': t('source.sort_name'),
    'status': t('source.sort_status'),
    'created': t('source.sort_created'),
  }
  return labels[sourceSortField.value]
})

const modeToggleTitle = computed(() => {
  if (headerMode.value === 'add') {
    return hasActiveSourceSearch.value
      ? t('source.search_active_tip', trimmedSourceSearch.value)
      : t('source.switch_search')
  }
  return t('source.switch_add')
})

function sourceCheckedText(source: Source): string {
  if (!source.last_checked_at) return t('source.never_checked')
  return t('source.last_checked', formatDate(source.last_checked_at))
}

function sourceHealthAriaLabel(source: Source): string {
  if (!source.enabled) return t('source.health_paused')
  if (source.last_check_status === 'ok') return t('source.health_normal')
  if (source.last_check_status === 'error') return t('source.health_error')
  return t('source.health_unknown')
}

const tooltip = ref<{ visible: boolean; x: number; y: number; lines: { text: string; wrap: boolean }[] }>({ visible: false, x: 0, y: 0, lines: [] })

function showHealthTooltip(e: MouseEvent, source: Source) {
  const lines: { text: string; wrap: boolean }[] = []
  let statusText: string
  if (!source.enabled) statusText = t('source.health_paused')
  else if (source.last_check_status === 'ok') statusText = t('source.health_normal')
  else if (source.last_check_status === 'error') statusText = t('source.health_error')
  else statusText = t('source.health_unknown')
  lines.push({ text: t('source.tooltip_status') + statusText, wrap: false })
  const count = props.totalReleaseCounts[sourceKey(source)]
  if (count > 0) {
    lines.push({ text: t('source.tooltip_history') + t('source.recorded_versions', String(count)), wrap: false })
  }
  if (source.description) {
    lines.push({ text: t('source.tooltip_about') + source.description, wrap: true })
  }
  tooltip.value = { visible: true, x: e.clientX + 10, y: e.clientY + 10, lines }
  nextTick(() => {
    const el = document.querySelector('.source-health-tooltip') as HTMLElement | null
    if (!el) return
    const rect = el.getBoundingClientRect()
    let { x, y } = tooltip.value
    if (rect.right > window.innerWidth - 4) x = e.clientX - rect.width - 10
    if (rect.bottom > window.innerHeight - 4) y = e.clientY - rect.height - 10
    tooltip.value.x = Math.max(4, x)
    tooltip.value.y = Math.max(4, y)
  })
}

function hideHealthTooltip() {
  tooltip.value.visible = false
}
</script>

<template>
  <section class="tab-content">
    <div class="source-sticky-panel" :class="{ 'has-bulk-bar': selectionMode }">
      <div class="source-header">
        <div class="input-clear-wrap">
          <input
            v-if="headerMode === 'add'"
            v-model="urlInput"
            :placeholder="t('source.placeholder')"
            @keyup.enter="handleAdd"
          />
          <input
            v-else
            v-model="sourceSearch"
            :placeholder="t('source.search')"
            class="search-input"
          />
          <button
            v-if="headerMode === 'add' && urlInput"
            type="button"
            class="input-clear-btn"
            :title="t('input.clear')"
            @click="urlInput = ''"
          >✕</button>
          <button
            v-else-if="headerMode === 'search' && sourceSearch"
            type="button"
            class="input-clear-btn"
            :title="t('input.clear')"
            @click="sourceSearch = ''"
          >✕</button>
        </div>
        <button v-if="headerMode === 'add'" class="btn-add-source" :disabled="loading || !urlInput" @click="handleAdd">{{ t('source.add') }}</button>
        <div class="sort-group">
          <button type="button" class="sort-trigger" :aria-expanded="openSort" aria-haspopup="menu" @click.stop="openSort = !openSort; if (openSort) focusSortDropdown()" @keydown="handleSortTriggerKeydown">
            <span class="sort-direction-icon" aria-hidden="true">{{ sourceSortDirection === 'asc' ? '↑' : '↓' }}</span>
            <span>{{ sortLabelText }}</span>
            <svg class="sort-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
          </button>
          <div v-if="openSort" class="sort-dropdown" role="menu" @click.stop @keydown="handleSortDropdownKeydown">
            <button type="button" role="menuitem" :aria-selected="sourceSortField === opt.value" v-for="opt in sortFieldOptions" :key="opt.value" :class="{ selected: sourceSortField === opt.value }" @click="sourceSortField = opt.value; openSort = false">{{ opt.label }}</button>
            <div class="sort-dropdown-divider"></div>
            <button type="button" role="menuitem" :aria-selected="sourceSortDirection === opt.value" v-for="opt in sortDirectionOptions" :key="opt.value" :class="{ selected: sourceSortDirection === opt.value }" @click="sourceSortDirection = opt.value; openSort = false">{{ opt.label }}</button>
          </div>
        </div>
        <button class="btn-select" @click="selectionMode = !selectionMode; if (!selectionMode) clearSelection()">
          <svg class="btn-select-icon" width="13" height="13" aria-hidden="true"><use :href="selectionMode ? '/icons.svg#checkbox-checked-icon' : '/icons.svg#checkbox-icon'"/></svg>
          {{ selectionMode ? t('source.select_cancel') : t('source.select') }}
        </button>
        <button
          type="button"
          class="btn-mode-toggle"
          :class="{ 'has-active-search': headerMode === 'add' && hasActiveSourceSearch }"
          :title="modeToggleTitle"
          :aria-label="modeToggleTitle"
          @click="headerMode = headerMode === 'add' ? 'search' : 'add'"
        >
          <svg v-if="headerMode === 'add'" width="16" height="16"><use href="/icons.svg#search-icon"/></svg>
          <span v-else class="mode-toggle-plus" aria-hidden="true">+</span>
          <span v-if="headerMode === 'add' && hasActiveSourceSearch" class="mode-toggle-dot" aria-hidden="true"></span>
        </button>
      </div>
      <div v-if="selectionMode" class="bulk-bar">
        <span class="bulk-count">{{ t('source.bulk_count', String(selectedCount)) }}</span>
        <button class="btn-sm" @click="selectAllVisible()">
          <span class="bulk-btn-icon bulk-select-all-icon" aria-hidden="true">✓</span>
          <span>{{ t('source.bulk_select_all') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0" @click="clearSelectedSources()">
          <span class="bulk-btn-icon bulk-clear-icon" aria-hidden="true">✕</span>
          <span>{{ t('source.bulk_clear_selection') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkToggle(true)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#play-icon"/></svg>
          <span>{{ t('source.bulk_resume') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkToggle(false)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#pause-icon"/></svg>
          <span>{{ t('source.bulk_pause') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkMuteToggle(true)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#bell-off-icon"/></svg>
          <span>{{ t('source.bulk_mute') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkMuteToggle(false)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#bell-icon"/></svg>
          <span>{{ t('source.bulk_unmute') }}</span>
        </button>
        <button class="btn-sm btn-danger" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkRemove">
          <svg class="bulk-btn-icon"><use href="/icons.svg#trash-icon"/></svg>
          <span>{{ t('source.bulk_delete') }}</span>
        </button>
      </div>
    </div>
    <div class="source-list">
      <div v-if="props.sources.length === 0" class="empty">{{ t('source.empty') }}</div>
      <div v-else-if="hasActiveSourceSearch && sortedSources.length === 0" class="empty source-search-status">{{ t('source.search_empty') }}</div>
      <div v-for="source in sortedSources" :key="source.id" class="source-item" :class="{ 'source-highlight': source.id === highlightedId }">
      <div v-if="selectionMode" class="source-checkbox">
        <input type="checkbox" :checked="selectedSourceIds.has(source.id)" @change="toggleSelection(source.id)" />
      </div>
        <div class="source-main">
          <div class="source-info">
            <span v-if="props.showSourceTypeIcons" class="source-type-badge" :class="source.source_type" :title="source.source_type === 'huggingface' ? t('source.type_huggingface') : t('source.type_github')">
              <svg><use :href="source.source_type === 'huggingface' ? '/icons.svg#huggingface-icon' : '/icons.svg#github-mark'"/></svg>
            </span>
            <span class="source-name">{{ source.repo ? `${source.owner}/${source.repo}` : source.owner }}</span>
            <button class="btn-icon-link" @click="openSourceUrl(source)" @contextmenu.prevent.stop="handleContextMenu($event, source.source_type === 'huggingface' ? `https://huggingface.co/${source.owner}` : `https://github.com/${source.owner}/${source.repo}`)" :title="t('source.visit')">
              <svg><use href="/icons.svg#link-icon"/></svg>
            </button>
            <button class="btn-icon-link" @click="openSourceReleases(source)" :title="t('source.view_releases')">
              <svg><use href="/icons.svg#search-icon"/></svg>
            </button>
            <span v-if="source.enabled && source.muted" class="badge badge-muted">{{ t('source.muted') }}</span>
            <span v-else-if="source.enabled" class="badge badge-on">{{ t('source.enabled') }}</span>
            <span v-else class="badge badge-off">{{ t('source.paused') }}</span>
          </div>
          <div class="source-health">
            <span class="health-dot" :class="sourceHealthClass(source)" :aria-label="sourceHealthAriaLabel(source)" @mouseenter="showHealthTooltip($event, source)" @mouseleave="hideHealthTooltip"></span>
            <template v-if="source.enabled && source.last_check_status === 'ok'">
              <button
                v-if="unreadReleaseCount(source) > 0"
                class="source-pending-link"
                @click="openSourceUnreadReleases(source)"
              >
                {{ t('source.pending_updates', String(unreadReleaseCount(source))) }}
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
          <div v-if="source.last_check_status === 'error' && source.last_check_message" class="source-error" :title="translateError(source.last_check_message)">
            {{ translateError(source.last_check_message) }}
          </div>
        </div>
        <div class="source-actions">
          <button class="btn-icon-action btn-check" :disabled="props.polling || checkingId !== null" @click="handleCheckSingle(source.id)" :title="checkingId === source.id ? t('source.checking') : t('source.check')">
            <svg><use href="/icons.svg#refresh-icon"/></svg>
          </button>
          <button class="btn-icon-action" :class="source.enabled ? 'btn-pause' : 'btn-resume'" @click="handleToggle(source)" :title="source.enabled ? t('source.pause') : t('source.resume')">
            <svg><use :href="source.enabled ? '/icons.svg#pause-icon' : '/icons.svg#play-icon'"/></svg>
          </button>
          <div class="dropdown-more">
            <button type="button" class="btn-icon-action btn-more" :aria-expanded="openMoreId === source.id" aria-haspopup="menu" @click.stop="toggleMore(source.id)" @keydown="handleMoreKeydown($event, source.id)" :title="t('source.more')">
              <svg><use href="/icons.svg#more-icon"/></svg>
            </button>
            <div v-if="openMoreId === source.id" class="dropdown-more-panel" role="menu" @click.stop @keydown="handleMorePanelKeydown">
              <button type="button" role="menuitem" class="dropdown-item" :disabled="!source.enabled" :title="!source.enabled ? t('source.mute_disabled_tip') : ''" @click="handleMuteToggle(source)">
                <span class="dropdown-icon"><svg><use :href="source.muted ? '/icons.svg#bell-icon' : '/icons.svg#bell-off-icon'"/></svg></span>
                {{ source.muted ? t('source.unmute') : t('source.mute') }}
              </button>
              <button type="button" role="menuitem" class="dropdown-item dropdown-item-danger" @click="handleRemove(source.id)">
                <span class="dropdown-icon"><svg><use href="/icons.svg#trash-icon"/></svg></span>
                {{ t('source.delete') }}
              </button>
            </div>
          </div>
        </div>
      </div>
      <div v-if="hasActiveSourceSearch && sortedSources.length > 0" class="empty source-search-status">{{ t('source.search_result_count', String(sortedSources.length)) }}</div>
    </div>
    <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" @open="handleOpenLink" @copy="handleCopyLink" @close="closeContextMenu" />
    <div v-if="tooltip.visible" class="source-health-tooltip" :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }">
      <div v-for="(line, i) in tooltip.lines" :key="i" class="tooltip-line" :class="{ 'tooltip-line-wrap': line.wrap }">{{ line.text }}</div>
    </div>
  </section>
</template>
<style scoped>
/* 顶部单行工具区：选择模式下批量栏与主栏作为整体固定 */
.source-sticky-panel {
  position: sticky;
  top: 0;
  z-index: 10;
  margin-bottom: 12px;
  transition: top 0.15s ease;
}

:global(.app-main.is-scrolled .source-sticky-panel) {
  top: calc(-1 * var(--app-padding-y, 16px));
}

/* 与版本/日志 tab 的 .log-search-row 对齐：无内边距、无分隔线，保证三个 tab 输入框位置一致 */
.source-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0;
  background: var(--bg);
}

.source-header .input-clear-wrap {
  flex: 1;
  min-width: 160px;
}

.source-header .input-clear-wrap input {
  flex: 1;
  padding: 8px 12px;
  padding-right: 34px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 13px;
  outline: none;
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

.source-header .input-clear-wrap input:focus {
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}

.source-header .input-clear-wrap .search-input {
  max-width: none;
}

.btn-add-source {
  padding: 8px 14px;
  background: var(--ink);
  color: var(--on-ink);
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  white-space: nowrap;
  transition: background 0.15s ease, transform 0.1s ease;
}

.btn-add-source:hover:not(:disabled) {
  background: var(--ink-hover);
}

.btn-add-source:active:not(:disabled) {
  transform: translateY(1px);
}

.btn-add-source:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-mode-toggle {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface);
  color: var(--text-muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.12s;
}

.btn-mode-toggle:hover {
  background: var(--bg-subtle);
  border-color: var(--border-strong);
  color: var(--text);
}

.btn-mode-toggle.has-active-search {
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
  color: var(--primary);
}

.mode-toggle-dot {
  position: absolute;
  top: 5px;
  right: 5px;
  width: 7px;
  height: 7px;
  border: 2px solid var(--surface);
  border-radius: 999px;
  background: var(--primary);
}

.btn-mode-toggle.has-active-search:hover .mode-toggle-dot {
  border-color: var(--bg);
}

.btn-mode-toggle svg {
  width: 16px;
  height: 16px;
}

.mode-toggle-plus {
  font-size: 20px;
  line-height: 1;
  font-weight: 500;
}

/* 监控源列表 */
.source-list {
  display: flex;
  flex-direction: column;
}

.source-search-status {
  padding-top: 30px;
}

/* 搜索结果计数行不显示空状态图标 */
.source-search-status::before {
  content: none;
}

.source-item {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 8px;
  background: transparent;
  border-bottom: 1px solid var(--border);
  border-radius: var(--radius-sm);
  transition: background 0.12s ease;
}

.source-item:hover {
  background: var(--bg-subtle);
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
  font-size: 12px;
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
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease;
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

.source-health-tooltip {
  position: fixed;
  z-index: 10002;
  padding: 8px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  font-size: 12px;
  line-height: 1.6;
  pointer-events: none;
  max-width: 360px;
  overflow-wrap: break-word;
}

.tooltip-line {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tooltip-line-wrap {
  white-space: normal;
  word-break: break-word;
}

.health-dot {
  position: relative;
  width: 7px;
  height: 7px;
  border-radius: 999px;
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
  border-radius: 999px;
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
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--danger);
}

.badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 0;
  background: transparent;
  font-size: 12px;
  color: var(--text-muted);
}

.badge::before {
  content: '';
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: var(--text-faint);
  flex-shrink: 0;
}

.badge-on::before {
  background: var(--success);
}

.badge-off::before {
  background: var(--warning);
}

.source-actions {
  display: flex;
  gap: 6px;
  align-items: center;
  justify-content: flex-end;
  flex-shrink: 0;
}

/* 更多按钮下拉容器 */
.dropdown-more {
  position: relative;
  display: inline-flex;
}

/* 侧边弹出面板 */
.dropdown-more-panel {
  position: absolute;
  top: 100%;
  right: 0;
  margin-top: 4px;
  min-width: 140px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  z-index: 100;
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
  font-size: 13px;
  cursor: pointer;
  transition: background 0.1s;
  white-space: nowrap;
}

.dropdown-item:hover:not(:disabled) {
  background: var(--bg-subtle);
}

.dropdown-item:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.dropdown-icon {
  font-size: 14px;
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

/* 新增源高亮动画 */
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

/* 图标操作按钮（幽灵风格：默认仅图标，悬停浮现底色） */
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
  transition: background 0.12s ease, color 0.12s ease;
  flex-shrink: 0;
}

.btn-icon-action svg {
  width: 16px;
  height: 16px;
}

.btn-icon-action:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.btn-icon-action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-check:hover:not(:disabled),
.btn-pause:hover:not(:disabled),
.btn-resume:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}


.sort-group {
  position: relative;
  flex-shrink: 0;
}

.sort-trigger {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.12s, border-color 0.12s;
}

.sort-trigger:hover {
  background: var(--bg-subtle);
  border-color: var(--border-strong);
}

.sort-direction-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 13px;
  height: 13px;
  color: var(--primary);
  font-size: 13px;
  line-height: 1;
  font-weight: 700;
}

.sort-arrow {
  color: var(--text-muted);
}

.sort-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  z-index: 100;
  min-width: 140px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-md);
  padding: 4px;
}

.sort-dropdown button {
  display: block;
  width: 100%;
  padding: 5px 14px;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  text-align: left;
  border-radius: 4px;
  transition: background 0.1s;
}

.sort-dropdown button:hover {
  background: var(--bg-subtle);
}

.sort-dropdown button.selected {
  font-weight: 600;
  color: var(--primary);
}

.sort-dropdown-divider {
  height: 1px;
  margin: 4px 2px;
  background: var(--border);
}

.btn-select {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 6px 7px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.12s, border-color 0.12s;
  flex-shrink: 0;
}

.btn-select:hover {
  background: var(--bg-subtle);
  border-color: var(--border-strong);
}

/* 选择模式复选框 */
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

/* 批量操作栏 */
.bulk-bar {
  display: flex;
  gap: 6px;
  align-items: center;
  flex-wrap: wrap;
  padding: 8px 0;
  background: var(--bg-subtle);
  border-bottom: 1px solid var(--border);
  font-size: 12px;
}

.bulk-count {
  font-weight: 600;
  color: var(--text);
  margin-right: 4px;
}

.bulk-bar .btn-sm {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 7px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.12s;
}

.bulk-btn-icon {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
}

.bulk-select-all-icon,
.bulk-clear-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  line-height: 1;
  font-weight: 700;
}

.bulk-bar .btn-sm:hover:not(:disabled) {
  background: var(--bg-hover);
}

.bulk-bar .btn-sm:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.bulk-bar .btn-danger {
  color: var(--danger);
}

.bulk-bar .btn-danger:hover:not(:disabled) {
  background: var(--danger-soft-bg);
  border-color: var(--danger);
}
</style>
