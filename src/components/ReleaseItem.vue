<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch, type ComponentPublicInstance } from 'vue'
import { storeToRefs } from 'pinia'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import ImageLightbox from './common/ImageLightbox.vue'
import BaseTooltip from './ui/BaseTooltip.vue'
import ReleaseVideoCardBody from './release/ReleaseVideoCardBody.vue'
import ReleaseStandardCardBody from './release/ReleaseStandardCardBody.vue'
import { type NotificationStatus, type ReleaseInfo, setNotificationState, deleteRelease } from '../api/releases'
import { openReleaseUrl, copyTextToClipboard } from '../api/client'
import { t, getLocale } from '../i18n'
import { formatDate, formatDateNoSeconds, formatDateShort, isReadStatus, isUnreadStatus, statusClass, statusLabel, tokenizeQuery, SNOOZE_MINUTES, type BodyIndex } from '../utils'
import { collectQueryRanges, registerHighlightRanges, unregisterHighlightOwner, searchHighlightSupported, extractBodySnippet } from '../utils/searchHighlight'
import { releaseDisplayTitle, releaseImportanceText, releaseImportanceClass, canTranslateRelease, releaseTitleRedundant } from '../utils/releaseDisplay'
import { FLAG_ITEMS } from '../utils/releaseFlag'
import { formatCompactCount, formatVideoDuration, formatViewCount, parseVideoMeta, type VideoMeta } from '../utils/releaseMeta'
import { mediaUrlOrEmpty } from '../utils/imageProxy'
import { registerCloser, unregisterCloser, closeAllContextMenus, isTransientUiInvalidated } from '../composables/contextMenuBus'
import { track } from '../composables/useUsageTracking'
import { useLineClamp } from '../composables/useLineClamp'
import { useReleaseActions } from '../composables/useReleaseActions'
import { useReleaseTranslate } from '../composables/useReleaseTranslate'
import { getSourceTypeDef, type HfMetaView } from '../api/source-registry'
import { extractErrorMessage } from '../services/feedback'
import { useUiStore } from '../stores/ui'
import { useSettingsStore } from '../stores/settings'
import { useCatalogStore } from '../stores/catalog'
import { useAgentStore } from '../stores/agent'
import { ENTITY_DRAG_MIME } from './agent/agentChatUtils'

const props = defineProps<{
  release: ReleaseInfo
  highlighted?: boolean
  selected?: boolean
  selectable?: boolean
  checked?: boolean
  searchQuery?: string
  bodyIndex?: BodyIndex | null
}>()

const emit = defineEmits<{
  update: []
  'open-detail': [release: ReleaseInfo]
  'toggle-select': [releaseId: number, shiftKey: boolean]
}>()

const ui = useUiStore()
const settingsStore = useSettingsStore()
const catalog = useCatalogStore()
const agentStore = useAgentStore()

const { aiEnabled, showImportance, autoReadDetail } = storeToRefs(settingsStore)
const { agentEnabled } = storeToRefs(catalog)
const { panelOpen: agentPanelOpen } = storeToRefs(agentStore)

const showToast = (msg: string) => ui.showToast(msg)

const {
  isUpdating,
  flagged,
  flagColor,
  flagTitle,
  applyFlag,
  updateStatus,
  sendToAgent: handleSendToAgent,
  copyImage,
} = useReleaseActions({
  release: () => props.release,
  showToast,
  openAgentWorkspace: (seed) => agentStore.openWorkspace(seed),
  onUpdated: () => emit('update'),
  onBeforeAction: () => closeMenus(),
})

function onSelectCheckboxClick(e: MouseEvent) {
  emit('toggle-select', props.release.id, e.shiftKey)
}

const { translating, handleTranslateRelease } = useReleaseTranslate({
  release: () => props.release,
  showToast,
  onStart: () => { summaryContextMenu.value = null },
  onSuccess: () => emit('update'),
})

const previewKind = computed<'summary' | 'body' | null>(() => {
  if (props.release.ai_summary) return 'summary'
  if (props.release.body_translated || props.release.body) return 'body'
  return null
})

const previewContent = computed<string | null>(() => {
  switch (previewKind.value) {
    case 'summary': return props.release.ai_summary
    case 'body': return props.release.body_translated || props.release.body
    default: return null
  }
})

const canOpenDetail = computed(() => !!(props.release.body || props.release.body_translated))

function openDetail() {
  if (!canOpenDetail.value) return
  track('release.read_full')
  emit('open-detail', props.release)
}

// ── 摘要行级截断 ──
const {
  textRef: summaryTextRef,
  display: summaryDisplay,
  hasThirdLine: summaryHasThirdLine,
} = useLineClamp({
  text: () => previewContent.value,
  expandBtnSelector: '.release-expand-btn',
})

function bindSummaryTextEl(el: Element | ComponentPublicInstance | null) {
  summaryTextRef.value = el as HTMLElement | null
}

// ── 悬浮提示（均交由 BaseTooltip 防溢出与失效守卫）──
const summaryTooltip = ref<{ x: number; y: number; text: string } | null>(null)
const hfHoverTooltip = ref<{ visible: boolean; x: number; y: number } | null>(null)

function isSummaryTruncated(el: HTMLElement): boolean {
  return el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1
}

function handleSummaryEnter(e: MouseEvent, summary: string | null) {
  if (!summary || isTransientUiInvalidated()) return
  const el = e.currentTarget as HTMLElement
  if (summaryDisplay.value === null && !isSummaryTruncated(el)) return
  summaryTooltip.value = { x: e.clientX, y: e.clientY, text: summary }
}

function handleSummaryMove(e: MouseEvent) {
  if (!summaryTooltip.value) return
  summaryTooltip.value = { x: e.clientX, y: e.clientY, text: summaryTooltip.value.text }
}

function handleSummaryFocus(e: FocusEvent, summary: string | null) {
  if (!summary || isTransientUiInvalidated()) return
  const el = e.currentTarget as HTMLElement
  if (summaryDisplay.value === null && !isSummaryTruncated(el)) return
  const rect = el.getBoundingClientRect()
  summaryTooltip.value = { x: rect.left, y: rect.bottom, text: summary }
}

function hideSummaryTooltip() {
  summaryTooltip.value = null
}

// ── 右键菜单与旗标菜单 ──
const contextMenu = ref<{ x: number; y: number; url: string; releaseId: number; onCover?: boolean } | null>(null)
const summaryContextMenu = ref<{ x: number; y: number; text: string } | null>(null)
const flagMenu = ref<{ x: number; y: number } | null>(null)
const flagButtonRef = ref<HTMLButtonElement | null>(null)
let flagMenuKeyboardOpened = false

function closeMenus() {
  if (flagMenu.value && flagMenuKeyboardOpened) {
    flagMenuKeyboardOpened = false
    void nextTick(() => flagButtonRef.value?.focus())
  }
  contextMenu.value = null
  flagMenu.value = null
  summaryContextMenu.value = null
  summaryTooltip.value = null
  hfHoverTooltip.value = null
}

const canTranslate = computed(() => canTranslateRelease(props.release, aiEnabled.value))

function sendToAgentItem(): ContextMenuItem | null {
  if (!agentEnabled.value) return null
  return { id: 'sendToAgent', label: t('agent.send_to') }
}

const summaryMenuItems = computed<ContextMenuItem[]>(() => {
  const items: ContextMenuItem[] = []
  if (canOpenDetail.value) items.push({ id: 'readFull', label: t('release.read_full') })
  items.push({ id: 'copyContent', label: t('context.copy_content') })
  if (canTranslate.value) items.push({ id: 'translate', label: t('context.translate') })
  const agentItem = sendToAgentItem()
  if (agentItem) items.push(agentItem)
  return items
})

const DEFAULT_FLAG = 1

const releaseMenuItems = computed<ContextMenuItem[]>(() => {
  const items: ContextMenuItem[] = [
    { id: 'openLink', label: t('context.open') },
    { id: 'copyLink', label: t('context.copy_link') },
  ]
  if (contextMenu.value?.onCover && coverSourceUrl.value) {
    items.push({ id: 'copyCoverImage', label: t('context.copy_cover_image') })
    items.push({ id: 'copyCoverLink', label: t('context.copy_cover_image_link') })
    items.push({ id: 'openCoverImage', label: t('context.open_cover_image') })
  }
  items.push({ id: 'divider-actions', label: '', divider: true })
  const agentItem = sendToAgentItem()
  if (agentItem) items.push(agentItem)
  items.push({ id: 'deleteRelease', label: t('context.delete_release') })
  return items
})

function handleSummaryContextMenu(e: MouseEvent, text: string | null) {
  if (!text) return
  closeAllContextMenus()
  summaryContextMenu.value = { x: e.clientX, y: e.clientY, text }
}

async function handleCopySummary() {
  if (!summaryContextMenu.value?.text) return
  track('release.copy')
  try {
    await copyTextToClipboard(summaryContextMenu.value.text)
  } catch (e: unknown) {
    showToast(t('release.copy_failed') + extractErrorMessage(e))
  }
  summaryContextMenu.value = null
}

function handleSummaryMenuAction(actionId: string) {
  if (actionId === 'readFull') openDetail()
  else if (actionId === 'copyContent') void handleCopySummary()
  else if (actionId === 'translate') void handleTranslateRelease()
  else if (actionId === 'sendToAgent') handleSendToAgent()
}

async function handleDeleteRelease() {
  const releaseId = contextMenu.value?.releaseId
  if (releaseId === undefined) return
  closeMenus()
  track('release.delete')
  isUpdating.value = true
  try {
    await deleteRelease(releaseId)
    showToast(t('release.deleted_toast'))
    emit('update')
  } catch (e: unknown) {
    showToast(t('release.delete_failed') + extractErrorMessage(e))
  } finally {
    isUpdating.value = false
  }
}

async function handleReleaseMenuAction(actionId: string) {
  if (actionId === 'openLink') handleOpenLink()
  else if (actionId === 'copyLink') void handleCopyLink()
  else if (actionId === 'copyCoverImage') void handleCopyCoverImage()
  else if (actionId === 'copyCoverLink') void handleCopyCoverLink()
  else if (actionId === 'openCoverImage') handleOpenCoverImage()
  else if (actionId === 'sendToAgent') handleSendToAgent()
  else if (actionId === 'deleteRelease') void handleDeleteRelease()
}

function toggleFlag() {
  void applyFlag(flagged.value ? 0 : DEFAULT_FLAG)
}

function handleDragStart(e: DragEvent) {
  if (!agentEnabled.value || !agentPanelOpen.value) return
  e.dataTransfer?.setData(ENTITY_DRAG_MIME, JSON.stringify({ kind: 'release', id: props.release.id }))
  if (e.dataTransfer) e.dataTransfer.effectAllowed = 'copy'
}

function releaseContextMenu(e: MouseEvent, url: string, onCover = false) {
  closeAllContextMenus()
  contextMenu.value = { x: e.clientX, y: e.clientY, url, releaseId: props.release.id, onCover }
}

function openFlagMenuAt(x: number, y: number, byKeyboard = false) {
  closeAllContextMenus()
  flagMenuKeyboardOpened = byKeyboard
  flagMenu.value = { x, y }
}

function openFlagMenu(e: MouseEvent) {
  openFlagMenuAt(e.clientX, e.clientY)
}

function handleFlagTriggerKeydown(e: KeyboardEvent) {
  if (e.key !== 'ArrowDown' && e.key !== 'ContextMenu' && !(e.shiftKey && e.key === 'F10')) return
  e.preventDefault()
  e.stopPropagation()
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  openFlagMenuAt(rect.left, rect.bottom, true)
}

const flagMenuItems = computed<ContextMenuItem[]>(() => {
  const items: ContextMenuItem[] = FLAG_ITEMS.map((f) => ({
    id: `flag-${f.value}`,
    label: t(f.labelKey),
    iconHref: '/icons.svg#flag-tag-icon',
    color: f.color,
    checked: props.release.flag === f.value,
  }))
  if (flagged.value) items.push({ id: 'flag-clear', label: t('release.flag_clear') })
  return items
})

function handleFlagMenuAction(actionId: string) {
  if (actionId === 'flag-clear') void applyFlag(0)
  else if (actionId.startsWith('flag-')) void applyFlag(Number(actionId.slice(5)))
}

async function handleCopyLink() {
  track('release.copy')
  try {
    await copyTextToClipboard(contextMenu.value!.url)
  } catch (e: unknown) {
    showToast(t('release.copy_failed') + extractErrorMessage(e))
  }
  closeMenus()
}

function handleOpenLink() {
  void openReleaseUrl(contextMenu.value!.url)
  closeMenus()
}

async function handleCopyCoverImage() {
  const url = coverSourceUrl.value
  closeMenus()
  await copyImage(url)
}

async function handleCopyCoverLink() {
  const url = coverSourceUrl.value
  closeMenus()
  if (!url) return
  track('release.copy')
  try {
    await copyTextToClipboard(url)
  } catch (e: unknown) {
    showToast(t('release.copy_failed') + extractErrorMessage(e))
  }
}

const coverLightbox = ref<{ src: string; originalUrl: string } | null>(null)
function handleOpenCoverImage() {
  const originalUrl = coverSourceUrl.value
  const src = youtubeThumb.value
  closeMenus()
  if (!originalUrl || !src) return
  track('release.open')
  coverLightbox.value = { src, originalUrl }
}

async function updateReleaseStatus(_release: ReleaseInfo, status: NotificationStatus, minutes?: number) {
  await updateStatus(status, minutes)
}

async function handleGoRelease(release: ReleaseInfo) {
  track('release.open')
  if (isReadStatus(release.notification_status) || autoReadDetail.value === 'off') {
    void openReleaseUrl(release.html_url)
    return
  }
  isUpdating.value = true
  try {
    await setNotificationState(release.id, 'clicked')
    emit('update')
    void openReleaseUrl(release.html_url)
  } catch (e: unknown) {
    showToast(t('release.status_failed') + extractErrorMessage(e))
  } finally {
    isUpdating.value = false
  }
}

// ── HuggingFace 元数据 ──
const hfMeta = computed<HfMetaView | null>(() =>
  getSourceTypeDef(props.release.source_type)?.renderMeta?.(props.release) ?? null,
)

const hfTooltip = computed<string | null>(() => {
  if (!hfMeta.value) return null
  const parts: string[] = []
  if (hfMeta.value.pipeline_tag) parts.push(`${t('release.hf_pipeline_tag')}: ${hfMeta.value.pipeline_tag}`)
  if (hfMeta.value.downloads != null) parts.push(`${t('release.hf_downloads')}: ${formatCompactCount(hfMeta.value.downloads)}`)
  if (hfMeta.value.likes != null) parts.push(`${t('release.hf_likes')}: ${formatCompactCount(hfMeta.value.likes)}`)
  if (hfMeta.value.gated) parts.push('gated')
  return parts.length ? parts.join('  ·  ') : null
})

function showHfTooltip(e: MouseEvent) {
  if (!hfTooltip.value || isTransientUiInvalidated()) return
  hfHoverTooltip.value = { visible: true, x: e.clientX, y: e.clientY }
}
function moveHfTooltip(e: MouseEvent) {
  if (!hfHoverTooltip.value) return
  hfHoverTooltip.value.x = e.clientX
  hfHoverTooltip.value.y = e.clientY
}
function hideHfTooltip() {
  hfHoverTooltip.value = null
}

// ── 源布局与视频元数据 ──
const showReleaseRepo = computed(() => getSourceTypeDef(props.release.source_type)?.showRepoPrefix === true)
const isYoutube = computed(() => getSourceTypeDef(props.release.source_type)?.youtubeLayout === true)
const showReleaseTag = computed(() => getSourceTypeDef(props.release.source_type)?.showTag !== false)
const cardTitle = computed(() => {
  if (isYoutube.value) return ''
  return releaseTitleRedundant(props.release) ? '' : releaseDisplayTitle(props.release)
})
const youtubeChannelName = computed(() => {
  const def = getSourceTypeDef(props.release.source_type)
  return def?.displayName?.(props.release.owner, props.release.repo, props.release.source_description)
    ?? `${props.release.owner}/${props.release.repo}`
})
const youtubeMeta = computed<VideoMeta | null>(() =>
  isYoutube.value ? parseVideoMeta(props.release.extra_metadata) : null,
)

const LOW_TIER_THUMB = /\/vi\/[^/]+\/(?:default|mqdefault|hqdefault)\.jpg$/i
const maxresCoverUrl = computed<string | null>(() => {
  if (props.release.source_type !== 'youtube') return null
  const id = (props.release.tag_name ?? '').trim()
  return /^[A-Za-z0-9_-]{6,32}$/.test(id) ? `https://i.ytimg.com/vi/${id}/maxresdefault.jpg` : null
})
const coverDisplayUrl = computed<string | null>(() => {
  const stored = youtubeMeta.value?.thumbnail ?? null
  const upgraded = maxresCoverUrl.value
  if (!upgraded) return stored
  if (stored && !LOW_TIER_THUMB.test(stored)) return stored
  return upgraded
})
const youtubeThumb = computed(() => mediaUrlOrEmpty(coverDisplayUrl.value))
const coverSourceUrl = computed(() => youtubeMeta.value?.thumbnail ?? maxresCoverUrl.value)
const youtubeIsLive = computed(() => youtubeMeta.value?.kind === 'live')
const youtubeDuration = computed(() => formatVideoDuration(youtubeMeta.value?.duration ?? null))
const youtubeViewCount = computed(() => youtubeMeta.value?.viewCount ?? null)
const youtubeViewText = computed(() => {
  const n = youtubeViewCount.value
  if (n == null) return null
  return t('release.yt_views', formatViewCount(n, getLocale()))
})
const youtubeViewTitle = computed(() =>
  youtubeViewCount.value != null ? String(youtubeViewCount.value) : '',
)

// ── 搜索命中高亮 ──
const cardEl = ref<HTMLElement | null>(null)
const highlightOwner = {}
const HIGHLIGHT_SKIP = 'button, time, .status-inline, .release-importance-chip, .release-flag-chip, .release-header-right, .release-hit-snippet, .md-code-lang'

function applySearchHighlight() {
  const tokens = tokenizeQuery(props.searchQuery ?? '')
  if (tokens.length === 0 || !searchHighlightSupported()) {
    unregisterHighlightOwner(highlightOwner)
    return
  }
  void nextTick(() => {
    const el = cardEl.value
    if (!el) return
    registerHighlightRanges(highlightOwner, collectQueryRanges(el, tokens, HIGHLIGHT_SKIP))
  })
}

watch(
  [
    () => props.searchQuery,
    previewContent,
    summaryDisplay,
    cardTitle,
    youtubeChannelName,
    () => props.release.tag_name,
    () => props.release.owner,
    () => props.release.repo,
  ],
  applySearchHighlight,
  { immediate: true },
)

const renderedPreview = computed(() =>
  previewKind.value === 'summary' ? (summaryDisplay.value ?? previewContent.value) : previewContent.value,
)

const bodyHitSnippet = computed(() => {
  const q = props.searchQuery ?? ''
  if (!q.trim()) return null
  const visible = `${props.release.owner}/${props.release.repo} ${props.release.tag_name} ${cardTitle.value} ${youtubeChannelName.value} ${renderedPreview.value ?? ''}`
  const indexed = props.bodyIndex?.get(props.release.id)
  const fullSummary = previewKind.value === 'summary' ? previewContent.value : null
  return extractBodySnippet(
    visible,
    [props.release.body_translated, props.release.body, indexed?.[1], indexed?.[0], fullSummary],
    q,
  )
})

onMounted(() => {
  registerCloser(closeMenus)
  document.addEventListener('click', closeMenus)
})
onUnmounted(() => {
  unregisterCloser(closeMenus)
  document.removeEventListener('click', closeMenus)
  unregisterHighlightOwner(highlightOwner)
})
</script>

<template>
  <div
    ref="cardEl"
    class="release-item"
    :class="[
      {
        'is-prerelease': release.prerelease,
        'release-highlight': props.highlighted,
        'release-selected': props.selected,
        'release-checked': props.selectable && props.checked,
      },
      showImportance ? releaseImportanceClass(release) : '',
    ]"
    :draggable="agentEnabled && agentPanelOpen && !props.selectable"
    @dragstart="handleDragStart"
  >
    <div class="release-header">
      <div class="release-heading">
        <label v-if="props.selectable" class="release-select-box" @click.stop>
          <input
            type="checkbox"
            :checked="props.checked"
            :aria-label="`${release.owner}/${release.repo} ${release.tag_name}`"
            @click="onSelectCheckboxClick"
          />
        </label>
        <span v-if="showReleaseRepo" class="release-repo" :title="`${release.owner}/${release.repo}`">{{ release.owner }}/{{ release.repo }}</span>
        <span v-else-if="isYoutube" class="release-repo release-repo-yt" :title="youtubeChannelName">{{ youtubeChannelName }}</span>
        <span
          v-if="showReleaseTag"
          class="release-tag"
          :class="{ 'release-tag-hf': !showReleaseRepo }"
          :title="hfTooltip ? undefined : release.tag_name"
          @mouseenter="showHfTooltip($event)"
          @mousemove="moveHfTooltip($event)"
          @mouseleave="hideHfTooltip"
        >{{ release.tag_name }}</span>
        <span v-if="flagColor" class="release-flag-chip" :style="{ color: flagColor }" :title="flagTitle">
          <svg><use href="/icons.svg#flag-tag-icon" /></svg>
        </span>
        <span v-if="showImportance && releaseImportanceText(release)" class="release-importance-chip" :class="releaseImportanceClass(release)">
          {{ releaseImportanceText(release) }}
        </span>
        <span class="status-inline" :class="statusClass(release.notification_status, release.snooze_until)">
          {{ statusLabel(release.notification_status, release.snooze_until) }}
        </span>
      </div>
      <div class="release-header-right">
        <span v-if="release.notification_status === 'snoozed' && release.snooze_until" class="release-status-meta">
          {{ t('release.snooze_until', formatDate(release.snooze_until)) }}
        </span>
        <button
          v-if="isReadStatus(release.notification_status)"
          class="btn-sm release-snooze-btn"
          :disabled="isUpdating"
          @click="updateReleaseStatus(release, 'snoozed', SNOOZE_MINUTES)"
        >{{ t('release.snooze') }}</button>
        <button
          v-if="isUnreadStatus(release.notification_status)"
          class="btn-sm btn-danger-soft"
          :disabled="isUpdating"
          @click="updateReleaseStatus(release, 'ignored')"
        >{{ t('release.ignore') }}</button>
        <button
          ref="flagButtonRef"
          class="btn-icon-link release-flag-action"
          :class="{ flagged }"
          :style="flagColor ? { color: flagColor } : undefined"
          :disabled="isUpdating"
          :title="flagTitle"
          aria-haspopup="menu"
          :aria-expanded="flagMenu !== null"
          @click="toggleFlag"
          @contextmenu.prevent.stop="openFlagMenu"
          @keydown="handleFlagTriggerKeydown"
        >
          <svg><use href="/icons.svg#flag-tag-icon" /></svg>
        </button>
        <button
          class="btn-icon-link release-link-action"
          :disabled="isUpdating"
          :title="t('release.open_link')"
          @click="handleGoRelease(release)"
          @contextmenu.prevent.stop="releaseContextMenu($event, release.html_url)"
        >
          <svg><use href="/icons.svg#link-icon" /></svg>
        </button>
        <time class="release-date" :datetime="release.published_at" :title="t('release.published_at', formatDate(release.published_at))">
          <span class="release-date-full">{{ t('release.published_at', formatDate(release.published_at)) }}</span>
          <span class="release-date-mid">{{ formatDateNoSeconds(release.published_at) }}</span>
          <span class="release-date-short">{{ formatDateShort(release.published_at) }}</span>
        </time>
      </div>
    </div>

    <ReleaseVideoCardBody
      v-if="isYoutube"
      :title="releaseDisplayTitle(release)"
      :preview-content="previewContent"
      :thumb-src="youtubeThumb"
      :duration="youtubeDuration"
      :is-live="youtubeIsLive"
      :view-text="youtubeViewText"
      :view-title="youtubeViewTitle"
      :can-open-detail="canOpenDetail"
      :is-updating="isUpdating"
      @open-link="handleGoRelease(release)"
      @cover-context-menu="releaseContextMenu($event, release.html_url, true)"
      @open-detail="openDetail"
      @summary-context-menu="handleSummaryContextMenu"
    />

    <ReleaseStandardCardBody
      v-else
      :card-title="cardTitle"
      :preview-kind="previewKind"
      :preview-content="previewContent"
      :summary-display="summaryDisplay"
      :summary-has-third-line="summaryHasThirdLine"
      :translating="translating"
      :can-open-detail="canOpenDetail"
      :bind-summary-text-el="bindSummaryTextEl"
      @summary-enter="handleSummaryEnter"
      @summary-move="handleSummaryMove"
      @summary-leave="hideSummaryTooltip"
      @summary-focus="handleSummaryFocus"
      @summary-blur="hideSummaryTooltip"
      @summary-context-menu="handleSummaryContextMenu"
      @open-detail="openDetail"
    />

    <div
      v-if="bodyHitSnippet && canOpenDetail"
      class="release-hit-snippet"
      :title="t('release.read_full')"
      @click="openDetail"
      @contextmenu.prevent.stop="handleSummaryContextMenu($event, previewContent)"
    >
      <svg class="release-hit-snippet-icon"><use href="/icons.svg#search-icon" /></svg>
      <span class="release-hit-snippet-label">{{ t('release.search_hit_body') }}</span>
      <span class="release-hit-snippet-text">
        <template v-for="(seg, i) in bodyHitSnippet" :key="i"><mark v-if="seg.hit" class="search-hit">{{ seg.text }}</mark><template v-else>{{ seg.text }}</template></template>
      </span>
    </div>
  </div>

  <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" :items="releaseMenuItems" @action="handleReleaseMenuAction" @close="closeMenus" />
  <ContextMenu v-if="flagMenu" :x="flagMenu.x" :y="flagMenu.y" :items="flagMenuItems" @action="handleFlagMenuAction" @close="closeMenus" />
  <ContextMenu v-if="summaryContextMenu" :x="summaryContextMenu.x" :y="summaryContextMenu.y" :items="summaryMenuItems" @action="handleSummaryMenuAction" @close="closeMenus" />

  <ImageLightbox
    v-if="coverLightbox"
    :src="coverLightbox.src"
    :original-url="coverLightbox.originalUrl"
    @close="coverLightbox = null"
  />

  <BaseTooltip
    :visible="summaryTooltip !== null"
    :x="summaryTooltip?.x"
    :y="summaryTooltip?.y"
    :max-width="520"
    tooltip-class="release-summary-tooltip"
    :text="summaryTooltip?.text ?? ''"
  />

  <BaseTooltip
    :visible="Boolean(hfHoverTooltip?.visible && hfTooltip)"
    :x="hfHoverTooltip?.x"
    :y="hfHoverTooltip?.y"
    :max-width="520"
    tooltip-class="release-summary-tooltip release-hf-tooltip"
    :text="hfTooltip ?? ''"
  />
</template>

<style scoped>
.release-item {
  position: relative;
  padding: 12px 14px;
  background: var(--surface);
  border-radius: var(--radius);
  border: 1px solid var(--border);
  transition: background-color var(--dur-hover) var(--ease-out);
  container-type: inline-size;
}

.release-item:hover {
  background: var(--hover-surface);
}

.release-item.release-highlight {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 25%, transparent);
  animation: release-focus-pulse 1.2s ease-out 2;
}

.release-item.release-selected {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 25%, transparent);
}

.release-item.release-checked {
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
}

.release-item.release-checked:hover {
  background: var(--primary-soft-bg);
}

.release-select-box {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  cursor: pointer;
  padding: 2px;
  margin-left: -2px;
}

@keyframes release-focus-pulse {
  0%, 100% { box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 25%, transparent); }
  50% { box-shadow: 0 0 0 6px color-mix(in srgb, var(--primary) 45%, transparent); }
}

.release-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
  margin-bottom: 6px;
}

.release-header-right {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.release-heading {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.release-repo {
  font-size: var(--fs-body);
  font-weight: 600;
  color: var(--text);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.release-tag {
  font-weight: 600;
  font-size: var(--fs-title);
  color: var(--primary);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.release-repo-yt {
  color: var(--text-muted);
  font-weight: 500;
}

.release-date {
  font-size: var(--fs-meta);
  color: var(--text-muted);
  white-space: nowrap;
}

.release-date-mid,
.release-date-short {
  display: none;
}

@container (max-width: 709px) {
  .release-date-full { display: none; }
  .release-date-mid { display: inline; }
}

@container (max-width: 619px) {
  .release-date-mid { display: none; }
  .release-date-short { display: inline; }
}

.release-tag-hf {
  cursor: help;
  text-decoration: underline dotted var(--primary-soft-border);
}

:global(.release-summary-tooltip) {
  padding: 8px 12px;
  font-size: var(--fs-body);
  line-height: 1.6;
  box-shadow: var(--shadow-lg);
  white-space: pre-wrap;
}

:global(.release-hf-tooltip) {
  white-space: nowrap;
}

@media (hover: hover) {
  .release-item .release-snooze-btn,
  .release-item :deep(.release-expand-btn) {
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--dur-hover) var(--ease-out) var(--delay-hover-out);
  }

  .release-item:hover .release-snooze-btn,
  .release-item:hover :deep(.release-expand-btn),
  .release-item:has(:focus-visible) .release-snooze-btn,
  .release-item:has(:focus-visible) :deep(.release-expand-btn) {
    opacity: 1;
    pointer-events: auto;
    transition: opacity var(--dur-hover) var(--ease-out);
  }

  body:has(.release-item:hover) .release-item:not(:hover) .release-snooze-btn,
  body:has(.release-item:hover) .release-item:not(:hover) :deep(.release-expand-btn) {
    transition: opacity calc(var(--dur-hover) / 2) var(--ease-out);
  }
}

.release-item .btn-sm:hover,
.release-item :deep(.btn-sm:hover) {
  background: var(--hover-surface-nested);
}

.release-status-meta {
  font-size: var(--fs-meta);
  color: var(--text-muted);
}

.release-link-action {
  width: 28px;
  height: 28px;
  margin: 0;
  border-radius: var(--radius);
}

.release-flag-chip {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}

.release-flag-chip svg {
  width: 13px;
  height: 13px;
}

.release-flag-action svg {
  width: 16px;
  height: 16px;
  color: var(--text-muted);
}

.release-flag-action.flagged svg {
  color: inherit;
}

.release-link-action svg {
  width: 16px;
  height: 16px;
}

.release-link-action:hover {
  background: var(--hover-surface-nested);
}

.release-hit-snippet {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  padding: 4px 8px;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
  font-size: var(--fs-meta);
  color: var(--text-muted);
  line-height: 1.5;
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out);
}

.release-hit-snippet:hover {
  background: var(--hover-surface-nested);
}

.release-hit-snippet-icon {
  width: 12px;
  height: 12px;
  flex-shrink: 0;
}

.release-hit-snippet-label {
  flex-shrink: 0;
  font-weight: 600;
}

.release-hit-snippet-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
