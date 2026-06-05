<script setup lang="ts">
import { ref, inject, onMounted, onUnmounted } from 'vue'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import { ShowToastKey } from '../injection-keys'
import { type NotificationStatus, type ReleaseInfo, setNotificationState, deleteRelease } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { t } from '../i18n'
import { formatDate, isReadStatus, isUnreadStatus, statusClass, statusLabel } from '../utils'
import { registerCloser, unregisterCloser, closeAllContextMenus } from '../composables/contextMenuBus'

const props = defineProps<{ release: ReleaseInfo }>()
const emit = defineEmits<{ update: [] }>()
const showToast = inject(ShowToastKey, () => {})

const snoozeMinutes = 24 * 60
const isUpdating = ref(false)

// ========== 摘要悬浮提示 ==========
const summaryTooltip = ref<{ x: number; y: number; text: string } | null>(null)

function isSummaryTruncated(el: HTMLElement): boolean {
  return el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1
}

function placeSummaryTooltip(x: number, y: number, text: string) {
  const maxWidth = 520
  const margin = 16
  const left = Math.max(margin, Math.min(x + 12, window.innerWidth - maxWidth - margin))
  summaryTooltip.value = { x: left, y: y + 12, text }
}

function handleSummaryEnter(e: MouseEvent, summary: string | null) {
  if (!summary) return
  const el = e.currentTarget as HTMLElement
  if (!isSummaryTruncated(el)) return
  placeSummaryTooltip(e.clientX, e.clientY, summary)
}

function handleSummaryMove(e: MouseEvent) {
  if (!summaryTooltip.value) return
  placeSummaryTooltip(e.clientX, e.clientY, summaryTooltip.value.text)
}

function handleSummaryFocus(e: FocusEvent, summary: string | null) {
  if (!summary) return
  const el = e.currentTarget as HTMLElement
  if (!isSummaryTruncated(el)) return
  const rect = el.getBoundingClientRect()
  placeSummaryTooltip(rect.left, rect.bottom, summary)
}

function hideSummaryTooltip() {
  summaryTooltip.value = null
}

// ========== 右键菜单 ==========
const contextMenu = ref<{ x: number; y: number; url: string; releaseId: number } | null>(null)

function closeMenus() {
  contextMenu.value = null
  summaryContextMenu.value = null
  summaryTooltip.value = null
}

const summaryContextMenu = ref<{ x: number; y: number; text: string } | null>(null)
const summaryMenuItems: ContextMenuItem[] = [
  { id: 'copySummary', label: t('context.copy_summary') },
]

const releaseMenuItems: ContextMenuItem[] = [
  { id: 'openLink', label: t('context.open') },
  { id: 'copyLink', label: t('context.copy_link') },
  { id: 'deleteRelease', label: t('context.delete_release') },
]

function handleSummaryContextMenu(e: MouseEvent, text: string | null) {
  if (!text) return
  closeAllContextMenus()
  summaryContextMenu.value = { x: e.clientX, y: e.clientY, text }
}

async function handleCopySummary() {
  if (!summaryContextMenu.value?.text) return
  try { await navigator.clipboard.writeText(summaryContextMenu.value.text) } catch { /* ignore */ }
  summaryContextMenu.value = null
}

function handleSummaryMenuAction(actionId: string) {
  if (actionId === 'copySummary') {
    handleCopySummary()
  }
}

async function handleDeleteRelease() {
  const releaseId = contextMenu.value?.releaseId
  if (releaseId === undefined) return
  closeMenus()
  isUpdating.value = true
  try {
    await deleteRelease(releaseId)
    showToast?.(t('release.deleted_toast'))
    emit('update')
  } catch (e: unknown) {
    showToast?.(t('release.delete_failed') + (e instanceof Error ? e.message : String(e)))
  } finally {
    isUpdating.value = false
  }
}

function handleReleaseMenuAction(actionId: string) {
  if (actionId === 'openLink') {
    handleOpenLink()
  } else if (actionId === 'copyLink') {
    handleCopyLink()
  } else if (actionId === 'deleteRelease') {
    handleDeleteRelease()
  }
}
onMounted(() => {
  registerCloser(closeMenus)
  document.addEventListener('click', closeMenus)
})
onUnmounted(() => {
  unregisterCloser(closeMenus)
  document.removeEventListener('click', closeMenus)
})

function releaseContextMenu(e: MouseEvent, url: string) {
  closeAllContextMenus()
  const releaseId = props.release.id
  contextMenu.value = { x: e.clientX, y: e.clientY, url, releaseId }
}

async function handleCopyLink() {
  try { await navigator.clipboard.writeText(contextMenu.value!.url) } catch { /* ignore */ }
  closeMenus()
}

function handleOpenLink() {
  openReleaseUrl(contextMenu.value!.url)
  closeMenus()
}

// ========== 操作处理 ==========
function statusSuccessMessage(status: NotificationStatus): string {
  if (status === 'snoozed') return t('release.snooze_scheduled')
  if (status === 'ignored') return t('release.notification_cancelled')
  return ''
}

async function updateReleaseStatus(release: ReleaseInfo, status: NotificationStatus, minutes?: number) {
  closeMenus()
  isUpdating.value = true
  try {
    await setNotificationState(release.id, status, minutes)
    const msg = statusSuccessMessage(status)
    if (msg) showToast?.(msg)
    emit('update')
  } catch (e: unknown) {
    showToast?.(t('release.status_failed') + (e instanceof Error ? e.message : String(e)))
  } finally {
    isUpdating.value = false
  }
}

async function handleGoRelease(release: ReleaseInfo) {
  if (isReadStatus(release.notification_status)) {
    openReleaseUrl(release.html_url)
    return
  }
  isUpdating.value = true
  openReleaseUrl(release.html_url)
  try {
    await setNotificationState(release.id, 'clicked')
    emit('update')
  } catch (e: unknown) {
    showToast?.(t('release.status_failed') + (e instanceof Error ? e.message : String(e)))
  } finally {
    isUpdating.value = false
  }
}

// ========== 显示辅助函数 ==========
function releaseDisplayTitle(release: ReleaseInfo): string {
  const name = release.release_name.trim()
  return name && name !== release.tag_name ? name : ''
}

function releaseImportanceText(release: ReleaseInfo): string {
  return release.ai_importance || ''
}

function releaseImportanceClass(release: ReleaseInfo): string {
  switch (release.ai_importance) {
    case '大': return 'release-importance-high'
    case '中': return 'release-importance-medium'
    case '小': return 'release-importance-low'
    default: return ''
  }
}
</script>

<template>
  <div class="release-item"
    :class="[{ 'is-prerelease': release.prerelease }, releaseImportanceClass(release)]">
    <div class="release-header">
      <div class="release-heading">
        <span class="release-repo">{{ release.owner }}/{{ release.repo }}</span>
        <span class="release-tag">{{ release.tag_name }}</span>
        <span class="release-dot">·</span>
        <span class="status-inline" :class="statusClass(release.notification_status)">{{ statusLabel(release.notification_status) }}</span>
        <span v-if="release.prerelease" class="badge badge-pre">{{ t('release.prerelease') }}</span>
      </div>
      <span class="release-date">{{ t('release.published_at', formatDate(release.published_at)) }}</span>
    </div>
    <div v-if="releaseDisplayTitle(release)" class="release-title">{{ releaseDisplayTitle(release) }}</div>
    <div v-if="release.ai_summary" class="release-summary-line">
      <span v-if="releaseImportanceText(release)" class="release-importance-chip" :class="releaseImportanceClass(release)">{{ releaseImportanceText(release) }}</span>
      <span
        class="release-summary-text"
        tabindex="0"
        @mouseenter="handleSummaryEnter($event, release.ai_summary)"
        @mousemove="handleSummaryMove"
        @mouseleave="hideSummaryTooltip"
        @focus="handleSummaryFocus($event, release.ai_summary)"
        @blur="hideSummaryTooltip"
        @contextmenu.prevent.stop="handleSummaryContextMenu($event, release.ai_summary)"
      >{{ release.ai_summary }}</span>
    </div>
    <div class="release-actions">
      <span v-if="release.notification_status === 'snoozed' && release.snooze_until" class="release-status-meta">{{ t('release.snooze_until', formatDate(release.snooze_until)) }}</span>
      <button class="btn-icon-link release-link-action" :disabled="isUpdating" @click="handleGoRelease(release)" @contextmenu.prevent.stop="releaseContextMenu($event, release.html_url)" :title="t('release.open_link')">
        <svg><use href="/icons.svg#link-icon"/></svg>
      </button>
      <button v-if="isReadStatus(release.notification_status)" class="btn-sm" :disabled="isUpdating" @click="updateReleaseStatus(release, 'snoozed', snoozeMinutes)">{{ t('release.snooze') }}</button>
      <button v-if="isUnreadStatus(release.notification_status)" class="btn-sm btn-danger-soft" :disabled="isUpdating" @click="updateReleaseStatus(release, 'ignored')">{{ t('release.ignore') }}</button>
    </div>
  </div>

  <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" :items="releaseMenuItems" @action="handleReleaseMenuAction" />
  <ContextMenu v-if="summaryContextMenu" :x="summaryContextMenu.x" :y="summaryContextMenu.y" :items="summaryMenuItems" @action="handleSummaryMenuAction" />

  <!-- 摘要悬浮提示 -->
  <div
    v-if="summaryTooltip"
    class="release-summary-tooltip"
    :style="{ left: summaryTooltip.x + 'px', top: summaryTooltip.y + 'px' }"
  >
    {{ summaryTooltip.text }}
  </div>
</template>
<style scoped>
/* 版本列表 */
.release-item {
  position: relative;
  padding: 10px 14px;
  background: var(--surface);
  border-radius: var(--radius);
  border: 1px solid var(--border);
  border-left: 4px solid var(--primary);
}

.release-item.is-prerelease {
  border-left-color: #9333ea;
}

.release-item.release-importance-high {
  border-left-color: var(--danger);
}

.release-item.release-importance-medium {
  border-left-color: #eab308;
}

.release-item.release-importance-low {
  border-left-color: var(--success);
}

.release-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
  margin-bottom: 6px;
}

.release-heading {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.release-repo {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.release-tag {
  font-weight: 600;
  font-size: 14px;
  color: var(--primary);
  flex-shrink: 0;
}

.release-dot {
  color: var(--text-muted);
  flex-shrink: 0;
}

.release-title {
  font-size: 13px;
  color: var(--text);
  margin: 2px 0 4px;
}

.release-date {
  font-size: 12px;
  color: var(--text-muted);
  margin-left: auto;
  white-space: nowrap;
}


.badge {
  display: inline-block;
  padding: 2px 8px;
  background: var(--bg);
  border-radius: 10px;
  font-size: 11px;
  color: var(--text-muted);
}

.badge-pre {
  background: #f3e8ff;
  color: #9333ea;
}

.release-summary-line {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-top: 4px;
  color: var(--text);
  line-height: 1.55;
}

.release-importance-chip {
  flex-shrink: 0;
  min-width: 22px;
  padding: 1px 6px;
  border-radius: 5px;
  text-align: center;
  font-size: 12px;
  font-weight: 700;
  background: var(--bg);
  color: var(--text-muted);
}

.release-importance-chip.release-importance-high {
  background: #fee2e2;
  color: var(--danger);
}

.release-importance-chip.release-importance-medium {
  background: #fef3c7;
  color: #d97706;
}

.release-importance-chip.release-importance-low {
  background: #dcfce7;
  color: var(--success);
}

.release-summary-text {
  min-width: 0;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  border-radius: 4px;
}

.release-summary-text:focus-visible {
  outline: 2px solid rgba(37, 99, 235, 0.35);
  outline-offset: 2px;
}

.release-summary-tooltip {
  position: fixed;
  z-index: 10002;
  max-width: min(520px, calc(100vw - 32px));
  padding: 9px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  box-shadow: 0 6px 20px rgba(0,0,0,0.14);
  color: var(--text);
  font-size: 13px;
  line-height: 1.6;
  overflow-wrap: anywhere;
  pointer-events: none;
}

.release-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 8px;
}

.status-inline {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  background: var(--bg);
  color: var(--text-muted);
}

.status-inline {
  padding: 1px 7px;
  font-size: 11px;
  flex-shrink: 0;
}

.release-status-meta {
  font-size: 12px;
  color: var(--text-muted);
}

.status-unread,
.status-pending {
  background: #dbeafe;
  color: var(--primary);
}

.status-read,
.status-clicked {
  background: #dcfce7;
  color: var(--success);
}

.status-ignored {
  background: #f3f4f6;
  color: #6b7280;
}

.status-snoozed {
  background: #fef3c7;
  color: var(--warning);
}

.release-link-action {
  width: 28px;
  height: 28px;
  margin: 0;
  border-radius: 6px;
}

.release-link-action svg {
  width: 16px;
  height: 16px;
}

.release-link-action:hover {
  background: var(--bg);
}

:global([data-theme="dark"] .release-summary-tooltip) {
  box-shadow: 0 6px 20px rgba(0,0,0,0.4);
}
</style>
