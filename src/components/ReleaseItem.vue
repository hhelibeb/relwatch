<script setup lang="ts">
import { ref, computed, inject, onMounted, onUnmounted, nextTick, watch } from 'vue'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import MarkdownContent from './common/MarkdownContent.vue'
import { ShowToastKey, AiEnabledKey } from '../injection-keys'
import { type NotificationStatus, type ReleaseInfo, setNotificationState, deleteRelease, translateRelease } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { t } from '../i18n'
import { formatDate, isReadStatus, isUnreadStatus, statusClass, statusLabel } from '../utils'
import { registerCloser, unregisterCloser, closeAllContextMenus } from '../composables/contextMenuBus'

const props = defineProps<{ release: ReleaseInfo }>()
const emit = defineEmits<{ update: [] }>()
const showToast = inject(ShowToastKey, () => {})
const aiEnabledRef = inject(AiEnabledKey, ref(false))

const aiEnabled = computed(() => aiEnabledRef.value)

const snoozeMinutes = 24 * 60
const isUpdating = ref(false)

// ========== 显示模式切换：摘要 / 译文 / 原文 ==========
type ViewMode = 'summary' | 'translated' | 'full'
const viewMode = ref<ViewMode>('summary')
const expanded = ref(false)
const translating = ref(false)

const availableModes = computed<{ mode: ViewMode; label: string }[]>(() => {
  const modes: { mode: ViewMode; label: string }[] = []
  if (props.release.ai_summary) modes.push({ mode: 'summary', label: t('release.view_summary') })
  // 译文：有译文或正在翻译时都显示该标签，翻译中显示“翻译中...”
  if (props.release.body_translated) {
    modes.push({ mode: 'translated', label: t('release.view_translated') })
  } else if (translating.value) {
    modes.push({ mode: 'translated', label: t('release.view_translating') })
  }
  // HuggingFace 源的 body 是模型 README（人类可读），作为“原文”展示
  if (props.release.body) modes.push({ mode: 'full', label: t('release.view_full') })
  return modes
})

const currentContent = computed<string | null>(() => {
  switch (viewMode.value) {
    case 'summary': return props.release.ai_summary
    case 'translated':
      // 翻译中且无译文时返回 null（模板会显示占位文案）
      if (translating.value && !props.release.body_translated) return null
      return props.release.body_translated
    case 'full': return props.release.body
    default: return null
  }
})

const hasLongContent = computed(() => {
  const c = currentContent.value
  // 估算超过 6 行（约 240 字符）时显示展开按钮
  return c !== null && c.length > 240
})

function switchMode(mode: ViewMode) {
  viewMode.value = mode
  expanded.value = false
}

// 展开/收起时保持视口稳定。展开长内容后滚动到下方点击「收起」，
// 内容高度骤减会让视口漂移。以被点击的收起按钮作为视口锚点：
// 记录按钮点击时的视口 top，收起渲染后 scrollBy 把按钮拉回原位置。
// 以按钮（而非卡片）为锚点，可保证多卡片乱序收起时仍稳定——
// 每次只锚定被点击的那个按钮，与卡片数量和顺序无关。
// 注意：滚动容器是 .app-main 而非 window，需向上查找实际滚动容器。
function toggleExpand(e: MouseEvent) {
  if (!expanded.value) {
    expanded.value = true
    return
  }
  const btn = e.currentTarget as HTMLElement | null
  const topBefore = btn ? btn.getBoundingClientRect().top : null
  expanded.value = false
  if (topBefore === null || !btn) return
  nextTick(() => {
    if (!btn) return
    const topAfter = btn.getBoundingClientRect().top
    const delta = topAfter - topBefore
    if (delta !== 0) {
      // 优先用按钮所在的滚动容器；找不到才回退到 window
      const scroller = btn.closest('.app-main') as HTMLElement | null
      if (scroller) {
        scroller.scrollTop += delta
      } else {
        window.scrollBy(0, delta)
      }
    }
  })
}

// 翻译完成后自动切换到译文视图：当 body_translated 从无到有，
// 且当前停留在原文视图（用户在此触发了翻译），自动切到译文标签。
watch(() => props.release.body_translated, (newVal, oldVal) => {
  if (newVal && !oldVal) {
    translating.value = false
    if (viewMode.value === 'full') {
      viewMode.value = 'translated'
      expanded.value = false
    }
  }
})

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
// 「翻译」选项仅在：当前为原文视图、无译文、AI 已启用 时出现
const canTranslate = computed(() =>
  viewMode.value === 'full'
  && !props.release.body_translated
  && aiEnabled.value
)
// 使用 computed 保证语言切换后右键菜单 label 实时更新
const summaryMenuItems = computed<ContextMenuItem[]>(() => {
  const items: ContextMenuItem[] = [{ id: 'copyContent', label: t('context.copy_content') }]
  if (canTranslate.value) {
    items.push({ id: 'translate', label: t('context.translate') })
  }
  return items
})

const releaseMenuItems = computed<ContextMenuItem[]>(() => [
  { id: 'openLink', label: t('context.open') },
  { id: 'copyLink', label: t('context.copy_link') },
  { id: 'deleteRelease', label: t('context.delete_release') },
])

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

async function handleTranslateRelease() {
  const releaseId = props.release.id
  summaryContextMenu.value = null
  // 立即进入翻译中状态并切到译文标签，让用户看到即时反馈
  translating.value = true
  viewMode.value = 'translated'
  expanded.value = false
  try {
    await translateRelease(releaseId)
    emit('update')
  } catch (e: unknown) {
    translating.value = false
    viewMode.value = 'full'
    showToast?.(t('release.translate_failed') + (e instanceof Error ? e.message : String(e)))
  }
}

function handleSummaryMenuAction(actionId: string) {
  if (actionId === 'copyContent') {
    handleCopySummary()
  } else if (actionId === 'translate') {
    handleTranslateRelease()
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
  // 默认视图优先级：摘要 > 译文 > 原文
  if (props.release.ai_summary) viewMode.value = 'summary'
  else if (props.release.body_translated) viewMode.value = 'translated'
  else if (props.release.body) viewMode.value = 'full'
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
  try {
    // 先标记已读再打开链接：保证状态与 UI 一致。若标记失败则不打开链接，
    // 避免出现“链接已打开但列表仍显示未读”的不同步状态。
    await setNotificationState(release.id, 'clicked')
    emit('update')
    openReleaseUrl(release.html_url)
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

// ========== HuggingFace 模型元数据 ==========
// HF 源的 release.extra_metadata 存储模型元数据 JSON（pipeline_tag/downloads/likes/gated/tags）
// body 列存模型 README（人类可读内容）
interface HfMeta {
  pipeline_tag: string | null
  downloads: number | null
  likes: number | null
  gated: boolean | null
}

const hfMeta = computed<HfMeta | null>(() => {
  if (props.release.source_type !== 'huggingface' || !props.release.extra_metadata) return null
  try {
    const obj = JSON.parse(props.release.extra_metadata)
    return {
      pipeline_tag: typeof obj.pipeline_tag === 'string' ? obj.pipeline_tag : null,
      downloads: typeof obj.downloads === 'number' ? obj.downloads : null,
      likes: typeof obj.likes === 'number' ? obj.likes : null,
      gated: typeof obj.gated === 'boolean' ? obj.gated : null,
    }
  } catch {
    return null
  }
})

function formatCount(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1).replace(/\.0$/, '') + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1).replace(/\.0$/, '') + 'K'
  return String(n)
}

// HF 源 tooltip 内容：pipeline_tag / downloads / likes / gated
const hfTooltip = computed<string | null>(() => {
  if (!hfMeta.value) return null
  const parts: string[] = []
  if (hfMeta.value.pipeline_tag) parts.push(`${t('release.hf_pipeline_tag')}: ${hfMeta.value.pipeline_tag}`)
  if (hfMeta.value.downloads != null) parts.push(`${t('release.hf_downloads')}: ${formatCount(hfMeta.value.downloads)}`)
  if (hfMeta.value.likes != null) parts.push(`${t('release.hf_likes')}: ${formatCount(hfMeta.value.likes)}`)
  if (hfMeta.value.gated) parts.push('gated')
  return parts.length ? parts.join('  ·  ') : null
})

const hfHoverTooltip = ref<{ visible: boolean; x: number; y: number } | null>(null)

function showHfTooltip(e: MouseEvent) {
  if (!hfTooltip.value) return
  hfHoverTooltip.value = { visible: true, x: e.clientX + 10, y: e.clientY + 10 }
}

function moveHfTooltip(e: MouseEvent) {
  if (!hfHoverTooltip.value) return
  hfHoverTooltip.value.x = e.clientX + 10
  hfHoverTooltip.value.y = e.clientY + 10
}

function hideHfTooltip() {
  hfHoverTooltip.value = null
}

// HF 源 tag_name 已含组织名（如 moonshotai/Kimi-K2.7-Code），不重复显示 release-repo 前缀
const showReleaseRepo = computed(() => props.release.source_type !== 'huggingface')

// ai_importance 存的是中文枚举（大/中/小），展示时映射到 i18n 文案，兼容英文界面
function releaseImportanceText(release: ReleaseInfo): string {
  switch (release.ai_importance) {
    case '大': return t('release.importance_high')
    case '中': return t('release.importance_medium')
    case '小': return t('release.importance_low')
    default: return ''
  }
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
        <span v-if="showReleaseRepo" class="release-repo">{{ release.owner }}/{{ release.repo }}</span>
        <span class="release-tag" :class="{ 'release-tag-hf': !showReleaseRepo }" @mouseenter="showHfTooltip($event)" @mousemove="moveHfTooltip($event)" @mouseleave="hideHfTooltip">{{ release.tag_name }}</span>
        <!-- 版本固有属性（重要性/预发布）贴版本号；状态（圆点+文字）放在分隔符后自成一体，避免圆点被误读为重要性指示 -->
        <span v-if="releaseImportanceText(release)" class="release-importance-chip" :class="releaseImportanceClass(release)">{{ releaseImportanceText(release) }}</span>
        <span v-if="release.prerelease" class="badge badge-pre">{{ t('release.prerelease') }}</span>
        <span class="status-inline" :class="statusClass(release.notification_status, release.snooze_until)">{{ statusLabel(release.notification_status, release.snooze_until) }}</span>
      </div>
      <div class="release-header-right">
        <span v-if="release.notification_status === 'snoozed' && release.snooze_until" class="release-status-meta">{{ t('release.snooze_until', formatDate(release.snooze_until)) }}</span>
        <button class="btn-sm" v-if="isReadStatus(release.notification_status)" :disabled="isUpdating" @click="updateReleaseStatus(release, 'snoozed', snoozeMinutes)">{{ t('release.snooze') }}</button>
        <button class="btn-sm btn-danger-soft" v-if="isUnreadStatus(release.notification_status, release.snooze_until)" :disabled="isUpdating" @click="updateReleaseStatus(release, 'ignored')">{{ t('release.ignore') }}</button>
        <button class="btn-icon-link release-link-action" :disabled="isUpdating" @click="handleGoRelease(release)" @contextmenu.prevent.stop="releaseContextMenu($event, release.html_url)" :title="t('release.open_link')">
          <svg><use href="/icons.svg#link-icon"/></svg>
        </button>
        <span class="release-date">{{ t('release.published_at', formatDate(release.published_at)) }}</span>
      </div>
    </div>
    <div v-if="releaseDisplayTitle(release)" class="release-title">{{ releaseDisplayTitle(release) }}</div>
    <div v-if="availableModes.length > 0" class="release-content">
      <div v-if="availableModes.length > 1" class="release-view-tabs">
        <button
          v-for="m in availableModes"
          :key="m.mode"
          class="release-view-tab"
          :class="{ active: viewMode === m.mode }"
          @click="switchMode(m.mode)"
        >{{ m.label }}</button>
      </div>
      <!-- 摘要模式：保持原有 2 行 clamp + 悬浮提示 -->
      <div v-if="viewMode === 'summary' && currentContent" class="release-summary-line">
        <span
          class="release-summary-text"
          tabindex="0"
          @mouseenter="handleSummaryEnter($event, currentContent)"
          @mousemove="handleSummaryMove"
          @mouseleave="hideSummaryTooltip"
          @focus="handleSummaryFocus($event, currentContent)"
          @blur="hideSummaryTooltip"
          @contextmenu.prevent.stop="handleSummaryContextMenu($event, currentContent)"
        >{{ currentContent }}</span>
      </div>
      <!-- 译文 / 原文模式：Markdown 渲染 + 长文本可展开 -->
      <div v-else-if="currentContent" class="release-body-text" :class="{ expanded }" @contextmenu.prevent.stop="handleSummaryContextMenu($event, currentContent)">
        <MarkdownContent :content="currentContent" />
      </div>
      <!-- 翻译中占位：译文尚未到达时显示加载提示 -->
      <div v-else-if="translating && viewMode === 'translated'" class="release-translating-hint">
        {{ t('release.translating_hint') }}
      </div>
      <button v-if="hasLongContent && viewMode !== 'summary'" class="btn-sm release-expand-btn" @click="toggleExpand">
        {{ expanded ? t('release.collapse') : t('release.expand') }}
      </button>
    </div>
  </div>

  <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" :items="releaseMenuItems" @action="handleReleaseMenuAction" @close="closeMenus" />
  <ContextMenu v-if="summaryContextMenu" :x="summaryContextMenu.x" :y="summaryContextMenu.y" :items="summaryMenuItems" @action="handleSummaryMenuAction" @close="closeMenus" />

  <!-- 摘要悬浮提示 -->
  <div
    v-if="summaryTooltip"
    class="release-summary-tooltip"
    :style="{ left: summaryTooltip.x + 'px', top: summaryTooltip.y + 'px' }"
  >
    {{ summaryTooltip.text }}
  </div>

  <!-- HF 模型元数据悬浮提示 -->
  <div
    v-if="hfHoverTooltip?.visible && hfTooltip"
    class="release-summary-tooltip release-hf-tooltip"
    :style="{ left: hfHoverTooltip.x + 'px', top: hfHoverTooltip.y + 'px' }"
  >
    {{ hfTooltip }}
  </div>
</template>
<style scoped>
/* 版本列表 */
.release-item {
  position: relative;
  padding: 12px 14px;
  background: var(--surface);
  border-radius: var(--radius);
  border: 1px solid var(--border);
  transition: border-color 0.15s ease;
}

.release-item:hover {
  border-color: var(--border-strong);
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

.release-title {
  font-size: 13px;
  color: var(--text);
  margin: 2px 0 4px;
}

.release-date {
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}


.badge {
  display: inline-block;
  padding: 0 5px;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  font-size: 11px;
  line-height: 16px;
  color: var(--text-muted);
}

.release-tag-hf {
  cursor: help;
  text-decoration: underline dotted var(--primary-soft-border);
}

.release-hf-tooltip {
  white-space: nowrap;
}

.release-summary-line {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-top: 4px;
  color: var(--text);
  line-height: 1.55;
}

.release-content {
  margin-top: 4px;
}

.release-view-tabs {
  display: inline-flex;
  gap: 2px;
  margin-bottom: 6px;
  padding: 2px;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
}

.release-view-tab {
  padding: 2px 10px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 11px;
  border-radius: 4px;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}

.release-view-tab:hover {
  color: var(--text);
}

.release-view-tab.active {
  background: var(--control-active);
  color: var(--text);
  font-weight: 600;
}

.release-body-text {
  margin-top: 4px;
  color: var(--text);
  font-size: 13px;
  line-height: 1.6;
  max-height: 9em;
  overflow: hidden;
  border-radius: 4px;
}

.release-body-text.expanded {
  max-height: none;
}

.release-expand-btn {
  margin-top: 4px;
  font-size: 11px;
}

.release-translating-hint {
  margin-top: 4px;
  padding: 12px;
  color: var(--text-muted);
  font-size: 13px;
  text-align: center;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
}

/* 重要性软色徽章：位于 header 行，三个内容视图常驻可见；规格与 badge-pre 一致 */
.release-importance-chip {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  padding: 0 6px;
  border-radius: var(--radius-xs);
  font-size: 11px;
  font-weight: 600;
  line-height: 16px;
}

.release-importance-chip.release-importance-high {
  background: var(--danger-soft-bg);
  color: var(--danger-soft-text);
}

.release-importance-chip.release-importance-medium {
  background: var(--warning-soft-bg);
  color: var(--warning-soft-text);
}

.release-importance-chip.release-importance-low {
  background: var(--success-soft-bg);
  color: var(--success-soft-text);
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
  outline: 2px solid var(--primary-soft-border);
  outline-offset: 2px;
}

.release-summary-tooltip {
  position: fixed;
  z-index: 10002;
  max-width: min(520px, calc(100vw - 32px));
  padding: 9px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  color: var(--text);
  font-size: 13px;
  line-height: 1.6;
  overflow-wrap: anywhere;
  pointer-events: none;
}

.status-inline {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 0;
  font-size: 12px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.status-inline::before {
  content: '';
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: var(--text-faint);
  flex-shrink: 0;
}

.release-status-meta {
  font-size: 12px;
  color: var(--text-muted);
}

.status-unread,
.status-pending {
  color: var(--primary);
}

.status-unread::before,
.status-pending::before {
  background: var(--primary);
}

.status-snoozed::before {
  background: var(--warning);
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
  background: var(--bg-hover);
}

</style>
