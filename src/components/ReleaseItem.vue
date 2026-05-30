<script setup lang="ts">
import { ref, inject, onMounted, onUnmounted } from 'vue'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import { ShowToastKey } from '../injection-keys'
import { type NotificationStatus, type ReleaseInfo, setNotificationState } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { t } from '../i18n'
import { formatDate, isReadStatus, isUnreadStatus, statusClass, statusLabel } from '../utils'

defineProps<{ release: ReleaseInfo }>()
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
const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)

function closeMenus() {
  contextMenu.value = null
  summaryContextMenu.value = null
  summaryTooltip.value = null
}

const summaryContextMenu = ref<{ x: number; y: number; text: string } | null>(null)
const summaryMenuItems: ContextMenuItem[] = [
  { id: 'copySummary', label: t('context.copy_summary') },
]

function handleSummaryContextMenu(e: MouseEvent, text: string | null) {
  if (!text) return
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
onMounted(() => document.addEventListener('click', closeMenus))
onUnmounted(() => document.removeEventListener('click', closeMenus))

function releaseContextMenu(e: MouseEvent, url: string) {
  contextMenu.value = { x: e.clientX, y: e.clientY, url }
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

  <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" @open="handleOpenLink" @copy="handleCopyLink" />
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
