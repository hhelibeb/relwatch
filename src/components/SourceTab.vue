<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, toRef } from 'vue'
import { storeToRefs } from 'pinia'
import {
  type Source,
  parseSourceUrl,
  addSource,
  removeSource,
  updateSource,
  batchUpdateSources,
  batchRemoveSources,
  buildYoutubeConfig,
  parseYtConfig,
  patchSourceConfig,
  getSourceTypeDef,
  sourceDisplayName,
  sourceSearchQuery,
  sourceRepoKey,
} from '../api/sources'
import { checkSingleSource, markSourceReleasesRead } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { useContextMenu } from '../composables/useContextMenu'
import { useDropdown } from '../composables/useDropdown'
import { useBulkSelection } from '../composables/useBulkSelection'
import { track } from '../composables/useUsageTracking'
import { confirmAction, extractErrorMessage } from '../services/feedback'
import { useUiStore } from '../stores/ui'
import { useCatalogStore } from '../stores/catalog'
import { useAgentStore } from '../stores/agent'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import EmptyState from './common/EmptyState.vue'
import ErrorBlock from './common/ErrorBlock.vue'
import SkeletonList from './common/SkeletonList.vue'
import BaseBulkBar from './ui/BaseBulkBar.vue'
import BaseTooltip from './ui/BaseTooltip.vue'
import SourceAddBar, { type HeaderMode } from './source/SourceAddBar.vue'
import SourceCard from './source/SourceCard.vue'
import { useSourceSortFilter, type SortDirection, type SourceSortField } from './source/useSourceSortFilter'
import { t, tm } from '../i18n'
import { ENTITY_DRAG_MIME } from './agent/agentChatUtils'

const props = defineProps<{
  sources: Source[]
  polling: boolean
  unreadReleaseCounts: Record<string, number>
  totalReleaseCounts: Record<string, number>
  showSourceTypeIcons: boolean
  loading?: boolean
  loadError?: string
}>()

const emit = defineEmits<{
  update: []
  checkResult: [count: number]
  checkBusy: [busy: boolean]
  openReleases: [query: string]
  openUnreadReleases: [query: string]
}>()

const ui = useUiStore()
const catalog = useCatalogStore()
const agentStore = useAgentStore()
const { activeTab } = storeToRefs(ui)
const { agentEnabled } = storeToRefs(catalog)
const { panelOpen: agentPanelOpen } = storeToRefs(agentStore)

const addBarRef = ref<InstanceType<typeof SourceAddBar> | null>(null)
const headerMode = ref<HeaderMode>('add')
const urlInput = ref('')
const addBusy = ref(false)
const checkingId = ref<number | null>(null)
const highlightedId = ref<number | null>(null)
const openMoreId = ref<number | null>(null)
const subscribeVideos = ref(true)
const subscribeLive = ref(true)
const subscribePosts = ref(false)
let highlightTimer: ReturnType<typeof setTimeout> | undefined

const {
  sourceSearch,
  trimmedSourceSearch,
  hasActiveSourceSearch,
  sourceSortField,
  sourceSortDirection,
  sortedSources,
  sortFieldOptions,
  sortDirectionOptions,
  sortLabelText,
} = useSourceSortFilter(toRef(props, 'sources'), toRef(props, 'unreadReleaseCounts'))

const openSort = ref(false)
const sortDropdown = useDropdown({
  openState: openSort,
  closedKey: false,
  hoverOpen: false,
})

const moreDropdown = useDropdown({
  openState: openMoreId,
  closedKey: null,
  hoverOpen: false,
})

const visibleSourceIds = computed(() => sortedSources.value.map((s) => s.id))
const bulkSelection = useBulkSelection<number>({
  visibleIds: visibleSourceIds,
  active: computed(() => activeTab.value === 'sources'),
})
const { selectionMode, selectedIds: selectedSourceIds, selectedCount } = bulkSelection
const bulkBusy = ref(false)
let pendingShiftClick = false

const { contextMenu, closeContextMenu, handleContextMenu, handleMenuAction } = useContextMenu()
const contextMenuItems = computed<ContextMenuItem[]>(() => [
  { id: 'open', label: t('context.open') },
  { id: 'copy', label: t('context.copy_link') },
])

function sourceKey(source: Source): string {
  return sourceRepoKey(source.source_type, source.owner, source.repo)
}

function unreadReleaseCount(source: Source): number {
  return props.unreadReleaseCounts[sourceKey(source)] || 0
}

function sourceHomeUrl(source: Source): string {
  return getSourceTypeDef(source.source_type)?.homeUrl(source.owner, source.repo)
    ?? `https://github.com/${source.owner}/${source.repo}`
}

function toggleHeaderMode() {
  headerMode.value = headerMode.value === 'add' ? 'search' : 'add'
  track(headerMode.value === 'add' ? 'source.switch_add' : 'source.switch_search')
}

function focusAddInput() {
  headerMode.value = 'add'
  void nextTick(() => addBarRef.value?.focusAddInput())
}

function selectSortField(field: SourceSortField) {
  sourceSortField.value = field
  sortDropdown.close()
  track('source.sort')
}

function selectSortDirection(dir: SortDirection) {
  sourceSortDirection.value = dir
  sortDropdown.close()
  track('source.sort')
}

function onSelectionCheckboxClick(e: MouseEvent) {
  pendingShiftClick = e.shiftKey
}

function toggleSelectionMode() {
  bulkSelection.toggleSelectionMode()
  track('source.select')
}

function toggleSelection(sourceId: number) {
  const shiftKey = pendingShiftClick
  pendingShiftClick = false
  bulkSelection.toggle(sourceId, shiftKey)
}

function selectAllVisible() {
  bulkSelection.selectAllVisible()
  track('source.bulk_select_all')
}

function clearSelectedSources() {
  bulkSelection.clearSelected()
  track('source.bulk_clear')
}

function handleDragStart(e: DragEvent, source: Source) {
  if (!agentEnabled.value || !agentPanelOpen.value) return
  e.dataTransfer?.setData(ENTITY_DRAG_MIME, JSON.stringify({ kind: 'source', id: source.id }))
  if (e.dataTransfer) e.dataTransfer.effectAllowed = 'copy'
}

function handleSendToAgent(source: Source) {
  moreDropdown.close()
  track('source.send_agent')
  agentStore.openWorkspace({ entities: [{ kind: 'source', id: source.id }] })
}

function openSourceUrl(source: Source) {
  track('source.open_url')
  void openReleaseUrl(sourceHomeUrl(source))
}

function openSourceReleases(source: Source) {
  track('source.view_releases')
  emit('openReleases', sourceSearchQuery(source))
}

function openSourceUnreadReleases(source: Source) {
  track('source.pending')
  emit('openUnreadReleases', sourceSearchQuery(source))
}

function sourceExists(parsed: { type: string; owner: string; repo: string }): boolean {
  return props.sources.some(
    (source) =>
      source.source_type === parsed.type
      && source.owner.toLowerCase() === parsed.owner.toLowerCase()
      && source.repo.toLowerCase() === parsed.repo.toLowerCase(),
  )
}

async function handleAdd() {
  const raw = urlInput.value.trim()
  if (!raw) return
  track('source.add')
  const parsed = parseSourceUrl(raw)
  if (!parsed) {
    ui.showToast(t('source.invalid_url'))
    return
  }
  if (sourceExists(parsed)) {
    ui.showToast(t('source.exists'))
    return
  }
  addBusy.value = true
  try {
    const def = getSourceTypeDef(parsed.type)
    const config = def?.hasConfigInput
      ? buildYoutubeConfig(subscribeVideos.value, subscribeLive.value, subscribePosts.value)
      : undefined
    if (def?.hasConfigInput && !subscribeVideos.value && !subscribeLive.value) {
      ui.showToast(t('source.require_subscribe'))
      return
    }
    const id = await addSource(parsed.type, parsed.owner, parsed.repo, config)
    if (id === 0) {
      ui.showToast(t('source.exists'))
      return
    }
    urlInput.value = ''
    subscribeVideos.value = true
    subscribeLive.value = true
    subscribePosts.value = false
    highlightedId.value = id
    clearTimeout(highlightTimer)
    highlightTimer = setTimeout(() => { highlightedId.value = null }, 2200)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(
      tm('source.add_failed', {
        source_type: parsed.type,
        owner: parsed.owner,
        repo: parsed.repo,
        error: extractErrorMessage(e),
      }),
    )
    emit('update')
  } finally {
    addBusy.value = false
  }
}

async function handleRemove(id: number) {
  track('source.remove')
  try {
    await removeSource(id)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.delete_failed') + extractErrorMessage(e))
  }
}

async function handleToggle(source: Source) {
  track(source.enabled ? 'source.pause' : 'source.resume')
  try {
    await updateSource(source.id, !source.enabled, source.poll_interval_minutes)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  }
  moreDropdown.close()
}

async function handleMuteToggle(source: Source) {
  if (!source.enabled) return
  track(source.muted ? 'source.unmute' : 'source.mute')
  try {
    await updateSource(source.id, source.enabled, source.poll_interval_minutes, !source.muted)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  }
  moreDropdown.close()
}

async function handleYtConfigToggle(source: Source, key: 'videos' | 'live', value: boolean) {
  track('source.yt_subscribe')
  const cfg = parseYtConfig(source.config)
  cfg[key] = value
  const config = patchSourceConfig(source.config, cfg)
  try {
    await updateSource(source.id, source.enabled, source.poll_interval_minutes, undefined, config)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  }
}

async function handleSourceFlagChange(source: Source, key: 'fetch_history' | 'check_prereleases', value: string) {
  const flag = value === 'on' ? true : value === 'off' ? false : undefined
  const config = patchSourceConfig(source.config, { [key]: flag })
  try {
    await updateSource(source.id, source.enabled, source.poll_interval_minutes, undefined, config)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  }
}

async function handleMarkSourceRead(source: Source) {
  moreDropdown.close()
  const count = unreadReleaseCount(source)
  if (count <= 0) return
  const confirmed = await confirmAction(
    t('source.mark_all_read_confirm', sourceDisplayName(source), String(count)),
    { title: t('release.mark_all_read'), kind: 'info' },
  )
  if (!confirmed) return
  track('source.mark_all_read')
  try {
    const updated = await markSourceReleasesRead(source.id)
    if (updated > 0) ui.showToast(t('release.bulk_marked_read', String(updated)))
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  }
}

async function handleCheckSingle(id: number) {
  if (props.polling || checkingId.value !== null) return
  track('source.check')
  checkingId.value = id
  emit('checkBusy', true)
  try {
    const result = await checkSingleSource(id)
    emit('update')
    emit('checkResult', result.new_releases.length)
  } catch (e: unknown) {
    ui.showToast(t('source.check_failed') + extractErrorMessage(e))
  } finally {
    checkingId.value = null
    emit('checkBusy', false)
  }
}

async function handleBulkToggle(enabled: boolean) {
  const ids = [...selectedSourceIds.value]
  if (ids.length === 0) return
  track(enabled ? 'source.bulk_resume' : 'source.bulk_pause')
  bulkBusy.value = true
  try {
    await batchUpdateSources(ids, enabled)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  } finally {
    bulkBusy.value = false
  }
}

async function handleBulkMuteToggle(muted: boolean) {
  const ids = [...selectedSourceIds.value]
  if (ids.length === 0) return
  track(muted ? 'source.bulk_mute' : 'source.bulk_unmute')
  bulkBusy.value = true
  try {
    await batchUpdateSources(ids, undefined, muted)
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.operation_failed') + extractErrorMessage(e))
  } finally {
    bulkBusy.value = false
  }
}

async function handleBulkRemove() {
  const ids = [...selectedSourceIds.value]
  if (ids.length === 0) return
  const confirmed = await confirmAction(t('source.bulk_delete_confirm', String(ids.length)), {
    title: t('source.delete'),
    kind: 'warning',
  })
  if (!confirmed) return
  track('source.bulk_delete')
  bulkBusy.value = true
  try {
    await batchRemoveSources(ids)
    clearSelectedSources()
    emit('update')
  } catch (e: unknown) {
    ui.showToast(t('source.delete_failed') + extractErrorMessage(e))
  } finally {
    bulkBusy.value = false
  }
}

// 健康点悬浮提示（由 BaseTooltip 统一防溢出与失效守卫）
const tooltip = ref<{ visible: boolean; x: number; y: number; lines: { text: string; wrap: boolean }[] }>({
  visible: false,
  x: 0,
  y: 0,
  lines: [],
})

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
  tooltip.value = { visible: true, x: e.clientX, y: e.clientY, lines }
}

function hideHealthTooltip() {
  tooltip.value.visible = false
}

function onDocumentClick() {
  moreDropdown.close()
  sortDropdown.close()
}

onMounted(() => document.addEventListener('click', onDocumentClick))
onUnmounted(() => {
  document.removeEventListener('click', onDocumentClick)
  if (highlightTimer) clearTimeout(highlightTimer)
})
</script>

<template>
  <section class="tab-content">
    <div class="source-sticky-panel" :class="{ 'has-bulk-bar': selectionMode }">
      <SourceAddBar
        ref="addBarRef"
        v-model:url-input="urlInput"
        v-model:source-search="sourceSearch"
        v-model:subscribe-videos="subscribeVideos"
        v-model:subscribe-live="subscribeLive"
        v-model:subscribe-posts="subscribePosts"
        :header-mode="headerMode"
        :sorted-count="sortedSources.length"
        :has-active-source-search="hasActiveSourceSearch"
        :trimmed-source-search="trimmedSourceSearch"
        :add-busy="addBusy"
        :open-sort="openSort"
        :source-sort-field="sourceSortField"
        :source-sort-direction="sourceSortDirection"
        :sort-label-text="sortLabelText"
        :sort-field-options="sortFieldOptions"
        :sort-direction-options="sortDirectionOptions"
        :selection-mode="selectionMode"
        :on-sort-trigger-click="(e) => sortDropdown.toggle(e, true)"
        :on-sort-trigger-keydown="(e) => sortDropdown.handleTriggerKeydown(e, true)"
        :on-sort-dropdown-keydown="sortDropdown.handleDropdownKeydown"
        @add="handleAdd"
        @toggle-header-mode="toggleHeaderMode"
        @toggle-selection-mode="toggleSelectionMode"
        @select-sort-field="selectSortField"
        @select-sort-direction="selectSortDirection"
      />
      <BaseBulkBar
        v-if="selectionMode"
        :selected-count="selectedCount"
        :selected-label="t('source.bulk_count', String(selectedCount))"
        @select-all="selectAllVisible"
        @clear="clearSelectedSources"
      >
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkToggle(true)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#play-icon" /></svg>
          <span>{{ t('source.bulk_resume') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkToggle(false)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#pause-icon" /></svg>
          <span>{{ t('source.bulk_pause') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkMuteToggle(true)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#bell-off-icon" /></svg>
          <span>{{ t('source.bulk_mute') }}</span>
        </button>
        <button class="btn-sm" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkMuteToggle(false)">
          <svg class="bulk-btn-icon"><use href="/icons.svg#bell-icon" /></svg>
          <span>{{ t('source.bulk_unmute') }}</span>
        </button>
        <button class="btn-sm btn-danger" :disabled="selectedCount === 0 || bulkBusy" @click="handleBulkRemove">
          <svg class="bulk-btn-icon"><use href="/icons.svg#trash-icon" /></svg>
          <span>{{ t('source.bulk_delete') }}</span>
        </button>
      </BaseBulkBar>
    </div>

    <div class="source-list">
      <SkeletonList v-if="props.loading && props.sources.length === 0" :rows="3" :label="t('common.loading')" />
      <ErrorBlock
        v-else-if="props.loadError && props.sources.length === 0"
        :message="props.loadError"
        :retry-label="t('common.retry')"
        @retry="emit('update')"
      />
      <EmptyState v-else-if="props.sources.length === 0" :title="t('source.empty')" :description="t('source.empty_desc')">
        <template #actions>
          <button type="button" class="btn-sm" @click="focusAddInput">{{ t('source.empty_cta') }}</button>
        </template>
      </EmptyState>
      <EmptyState
        v-else-if="hasActiveSourceSearch && sortedSources.length === 0"
        class="source-search-status"
        :title="t('source.search_empty')"
      >
        <template #actions>
          <button type="button" class="btn-sm" @click="sourceSearch = ''">{{ t('common.clear_search') }}</button>
        </template>
      </EmptyState>

      <SourceCard
        v-for="source in sortedSources"
        :key="source.id"
        :source="source"
        :highlighted="source.id === highlightedId"
        :selection-mode="selectionMode"
        :selected="selectedSourceIds.has(source.id)"
        :show-source-type-icons="props.showSourceTypeIcons"
        :unread-count="unreadReleaseCount(source)"
        :polling="props.polling"
        :checking-id="checkingId"
        :more-open="openMoreId === source.id"
        :agent-enabled="agentEnabled"
        :agent-panel-open="agentPanelOpen"
        :on-more-trigger-click="(e) => moreDropdown.toggle(e, source.id)"
        :on-more-trigger-keydown="(e) => moreDropdown.handleTriggerKeydown(e, source.id)"
        :on-more-dropdown-keydown="moreDropdown.handleDropdownKeydown"
        @drag-start="handleDragStart($event, source)"
        @checkbox-click="onSelectionCheckboxClick"
        @toggle-select="toggleSelection(source.id)"
        @open-url="openSourceUrl(source)"
        @context-menu-url="handleContextMenu($event, sourceHomeUrl(source))"
        @open-releases="openSourceReleases(source)"
        @open-unread-releases="openSourceUnreadReleases(source)"
        @show-health-tooltip="showHealthTooltip($event, source)"
        @hide-health-tooltip="hideHealthTooltip"
        @check-single="handleCheckSingle(source.id)"
        @toggle="handleToggle(source)"
        @yt-config-toggle="(k, v) => handleYtConfigToggle(source, k, v)"
        @flag-change="(k, v) => handleSourceFlagChange(source, k, v)"
        @mark-all-read="handleMarkSourceRead(source)"
        @mute-toggle="handleMuteToggle(source)"
        @send-to-agent="handleSendToAgent(source)"
        @remove="handleRemove(source.id)"
      />
      <div v-if="hasActiveSourceSearch && sortedSources.length > 0" class="source-search-result-count">
        {{ t('source.search_result_count', String(sortedSources.length)) }}
      </div>
    </div>

    <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" :items="contextMenuItems" @action="handleMenuAction" @close="closeContextMenu" />
    <BaseTooltip
      :visible="tooltip.visible"
      :x="tooltip.x"
      :y="tooltip.y"
      :max-width="360"
      tooltip-class="source-health-tooltip"
    >
      <div v-for="(line, i) in tooltip.lines" :key="i" class="tooltip-line" :class="{ 'tooltip-line-wrap': line.wrap }">{{ line.text }}</div>
    </BaseTooltip>
  </section>
</template>

<style scoped>
.source-sticky-panel {
  position: sticky;
  top: 0;
  z-index: var(--z-sticky);
  margin-bottom: 12px;
  transition: top var(--dur-state) var(--ease-out);
}

:global(.app-main.is-scrolled .source-sticky-panel) {
  top: calc(-1 * var(--app-padding-y));
}

.source-list {
  display: flex;
  flex-direction: column;
}

.source-search-status {
  padding-top: 30px;
}

.source-search-result-count {
  padding: 8px 0;
  text-align: center;
  color: var(--text-muted);
  font-size: var(--fs-meta);
}

:global(.source-health-tooltip) {
  padding: 9px 12px;
  box-shadow: var(--shadow-lg);
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
</style>
