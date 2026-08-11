<script setup lang="ts">
import { ref, computed, inject, onMounted, onUnmounted } from 'vue'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import MarkdownContent from './common/MarkdownContent.vue'
import { ShowToastKey, AiEnabledKey } from '../injection-keys'
import { type NotificationStatus, type ReleaseInfo, setNotificationState, deleteRelease } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { t, getLocale } from '../i18n'
import { formatDate, isReadStatus, isUnreadStatus, statusClass, statusLabel } from '../utils'
import { releaseDisplayTitle, releaseImportanceText, releaseImportanceClass, canTranslateRelease } from '../utils/releaseDisplay'
import { registerCloser, unregisterCloser, closeAllContextMenus } from '../composables/contextMenuBus'
import { track } from '../composables/useUsageTracking'
import { useLineClamp } from '../composables/useLineClamp'
import { useReleaseTranslate } from '../composables/useReleaseTranslate'
import { getSourceTypeDef, type HfMetaView } from '../api/source-registry'

const props = defineProps<{ release: ReleaseInfo }>()
const emit = defineEmits<{ update: []; 'open-detail': [release: ReleaseInfo] }>()
const showToast = inject(ShowToastKey, () => {})
const aiEnabledRef = inject(AiEnabledKey, ref(false))

const aiEnabled = computed(() => aiEnabledRef.value)

const snoozeMinutes = 24 * 60
const isUpdating = ref(false)

// ========== 卡片内容预览：摘要 > 译文 > 原文 ==========
// 卡片不再提供内容标签：摘要/译文/原文的切换集中在详情弹窗内进行，
// 点击正文预览或「阅读全文」按钮一步直达弹窗阅读全文。
const { translating, handleTranslateRelease } = useReleaseTranslate({
  release: () => props.release,
  showToast,
  // 翻译开始前关闭右键菜单，避免菜单残留
  onStart: () => { summaryContextMenu.value = null },
  onSuccess: () => emit('update'),
})

// 卡片只展示一种内容：摘要是概览的首选；无摘要时回退到译文/原文截断预览
// （HuggingFace 源的 body 是模型 README，作为“原文”展示）
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

// 只要有正文或译文可看，就提供「阅读全文」入口（按钮 + 右键菜单），
// 短内容也可能需要在弹窗中聚焦阅读、复制或逐版本导航
const canOpenDetail = computed(() => !!(props.release.body || props.release.body_translated))

function openDetail() {
  if (!canOpenDetail.value) return
  track('release.read_full')
  emit('open-detail', props.release)
}

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
  // 手动截断（summaryDisplay 非空）或 line-clamp 截断时都提供全文悬浮提示
  if (summaryDisplay.value === null && !isSummaryTruncated(el)) return
  placeSummaryTooltip(e.clientX, e.clientY, summary)
}

function handleSummaryMove(e: MouseEvent) {
  if (!summaryTooltip.value) return
  placeSummaryTooltip(e.clientX, e.clientY, summaryTooltip.value.text)
}

function handleSummaryFocus(e: FocusEvent, summary: string | null) {
  if (!summary) return
  const el = e.currentTarget as HTMLElement
  if (summaryDisplay.value === null && !isSummaryTruncated(el)) return
  const rect = el.getBoundingClientRect()
  placeSummaryTooltip(rect.left, rect.bottom, summary)
}

function hideSummaryTooltip() {
  summaryTooltip.value = null
}

// ========== 摘要行级截断与按钮定位 ==========
// 摘要超过两行时：前两行满行，第三行在按钮左侧精确截断（自带省略号），按钮悬浮其右侧，
// 两者不重叠（截断仅作用于显示层，tooltip/右键菜单仍用完整摘要）；
// 不足两行时按钮独立成行，避免悬浮遮挡正文。
const {
  textRef: summaryTextRef,
  display: summaryDisplay,
  hasThirdLine: summaryHasThirdLine,
} = useLineClamp({
  text: () => previewContent.value,
  expandBtnSelector: '.release-expand-btn',
})

// ========== 右键菜单 ==========
const contextMenu = ref<{ x: number; y: number; url: string; releaseId: number } | null>(null)

function closeMenus() {
  contextMenu.value = null
  summaryContextMenu.value = null
  summaryTooltip.value = null
}

const summaryContextMenu = ref<{ x: number; y: number; text: string } | null>(null)
// 「翻译」选项仅在：有原文、无译文、非 youtube 源、AI 已启用 时出现
const canTranslate = computed(() => canTranslateRelease(props.release, aiEnabled.value))
// 使用 computed 保证语言切换后右键菜单 label 实时更新
const summaryMenuItems = computed<ContextMenuItem[]>(() => {
  const items: ContextMenuItem[] = []
  if (canOpenDetail.value) {
    items.push({ id: 'readFull', label: t('release.read_full') })
  }
  items.push({ id: 'copyContent', label: t('context.copy_content') })
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
  track('release.copy')
  try { await navigator.clipboard.writeText(summaryContextMenu.value.text) } catch { /* ignore */ }
  summaryContextMenu.value = null
}

function handleSummaryMenuAction(actionId: string) {
  if (actionId === 'readFull') {
    openDetail()
  } else if (actionId === 'copyContent') {
    handleCopySummary()
  } else if (actionId === 'translate') {
    handleTranslateRelease()
  }
}

async function handleDeleteRelease() {
  const releaseId = contextMenu.value?.releaseId
  if (releaseId === undefined) return
  closeMenus()
  track('release.delete')
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
  track('release.copy')
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
  track(status === 'snoozed' ? 'release.snooze' : 'release.ignore')
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
  track('release.open')
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

// ========== HuggingFace 模型元数据 ==========
// HF 源的 release.extra_metadata 存储模型元数据 JSON（pipeline_tag/downloads/likes/gated/tags）
// body 列存模型 README（人类可读内容）；解析逻辑收敛在 source-registry 的 renderMeta。

const hfMeta = computed<HfMetaView | null>(() =>
  getSourceTypeDef(props.release.source_type)?.renderMeta?.(props.release) ?? null
)

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

// 展示行为按源类型注册表能力决定（github 显示 owner/repo 前缀；HF/YT 隐藏）
const showReleaseRepo = computed(() => getSourceTypeDef(props.release.source_type)?.showRepoPrefix === true)

// YouTube 源：channel_id 无展示意义，tag_name（videoId）同样隐藏；
// 显示真实频道名（source_description，兼容旧版 "YouTube channel: " 前缀）
const isYoutube = computed(() => getSourceTypeDef(props.release.source_type)?.youtubeLayout === true)
const showReleaseTag = computed(() => getSourceTypeDef(props.release.source_type)?.showTag !== false)
const youtubeChannelName = computed(() => {
  const def = getSourceTypeDef(props.release.source_type)
  return def?.displayName?.(props.release.owner, props.release.repo, props.release.source_description)
    ?? `${props.release.owner}/${props.release.repo}`
})

// YouTube 视频元数据（封面缩略图 / 类型 / 时长 / 播放量）
interface YoutubeMeta {
  thumbnail: string | null
  kind: 'video' | 'live' | null
  duration: string | null
  viewCount: number | null
}

const youtubeMeta = computed<YoutubeMeta | null>(() => {
  if (!isYoutube.value || !props.release.extra_metadata) return null
  try {
    const obj = JSON.parse(props.release.extra_metadata)
    // B 站旧数据封面可能是 http://，升级为 https 兼容 CSP（img-src 仅允许 https）
    const rawThumb = typeof obj.thumbnail === 'string' ? obj.thumbnail : null
    const thumbnail = rawThumb?.startsWith('http://') ? rawThumb.replace(/^http:\/\//, 'https://') : rawThumb
    return {
      thumbnail,
      kind: obj.kind === 'live' ? 'live' : obj.kind === 'video' ? 'video' : null,
      duration: typeof obj.duration === 'string' && obj.duration ? obj.duration : null,
      viewCount: typeof obj.view_count === 'number' ? obj.view_count : null,
    }
  } catch {
    return null
  }
})

const youtubeThumb = computed(() => youtubeMeta.value?.thumbnail ?? null)
const youtubeIsLive = computed(() => youtubeMeta.value?.kind === 'live')

// Data API 的 ISO 8601 时长（PT1H2M3S / PT12M34S）→ 人类可读（1:02:03 / 12:34）；RSS 模式无时长返回空
function formatYoutubeDuration(iso: string | null): string {
  if (!iso) return ''
  const m = iso.match(/^PT(?:\d+H)?(?:\d+M)?(?:\d+S)?$/)
  if (m) {
    const parts = iso.match(/^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$/)
    if (!parts) return ''
    const h = parts[1] ? parseInt(parts[1], 10) : 0
    const min = parts[2] ? parseInt(parts[2], 10) : 0
    const sec = parts[3] ? parseInt(parts[3], 10) : 0
    if (h > 0) return `${h}:${String(min).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
    return `${min}:${String(sec).padStart(2, '0')}`
  }
  // 非 ISO 格式（如 B 站 duration_text "12:34" / "1:02:33"）原样展示
  if (/^(?:\d+:)?\d{1,2}:\d{2}$/.test(iso)) return iso
  return ''
}

const youtubeDuration = computed(() => formatYoutubeDuration(youtubeMeta.value?.duration ?? null))

// 播放量格式化：中文环境用万/亿（123.4万 / 1.2亿），英文环境用 K/M（1.2M / 123K）。
// 阈值取“四舍五入后不溢出当前单位”的下限（如 99,950,000 → 1亿，而非 10000万）
function trimZero(v: number): string {
  return v.toFixed(1).replace(/\.0$/, '')
}

function formatViewCount(n: number): string {
  if (getLocale() === 'zh-CN') {
    if (n >= 99_950_000) return trimZero(n / 100_000_000) + '亿'
    if (n >= 9_950) return trimZero(n / 10_000) + '万'
    return String(n)
  }
  if (n >= 999_500) return trimZero(n / 1_000_000) + 'M'
  if (n >= 995) return trimZero(n / 1_000) + 'K'
  return String(n)
}

const youtubeViewCount = computed(() => youtubeMeta.value?.viewCount ?? null)
// 播放量文案（如“123.4万次播放”）；无数据（YouTube RSS 模式）返回 null 整行隐藏
const youtubeViewText = computed(() => {
  const n = youtubeViewCount.value
  if (n == null) return null
  return t('release.yt_views', formatViewCount(n))
})
// 悬浮提示显示精确数字（格式化后的 123.4万 不便精确阅读）
const youtubeViewTitle = computed(() =>
  youtubeViewCount.value != null ? String(youtubeViewCount.value) : ''
)
</script>

<template>
  <div class="release-item"
    :class="[{ 'is-prerelease': release.prerelease }, releaseImportanceClass(release)]">
    <div class="release-header">
      <div class="release-heading">
        <span v-if="showReleaseRepo" class="release-repo">{{ release.owner }}/{{ release.repo }}</span>
        <span v-else-if="isYoutube" class="release-repo release-repo-yt" :title="youtubeChannelName">{{ youtubeChannelName }}</span>
        <span v-if="showReleaseTag" class="release-tag" :class="{ 'release-tag-hf': !showReleaseRepo }" @mouseenter="showHfTooltip($event)" @mousemove="moveHfTooltip($event)" @mouseleave="hideHfTooltip">{{ release.tag_name }}</span>
        <!-- 版本固有属性（重要性/预发布）贴版本号；状态（圆点+文字）放在分隔符后自成一体，避免圆点被误读为重要性指示 -->
        <span v-if="releaseImportanceText(release)" class="release-importance-chip" :class="releaseImportanceClass(release)">{{ releaseImportanceText(release) }}</span>
        <span v-if="release.prerelease" class="pre-release-badge">{{ t('release.prerelease') }}</span>
        <span class="status-inline" :class="statusClass(release.notification_status, release.snooze_until)">{{ statusLabel(release.notification_status, release.snooze_until) }}</span>
      </div>
      <div class="release-header-right">
        <span v-if="release.notification_status === 'snoozed' && release.snooze_until" class="release-status-meta">{{ t('release.snooze_until', formatDate(release.snooze_until)) }}</span>
        <button class="btn-sm" v-if="isReadStatus(release.notification_status)" :disabled="isUpdating" @click="updateReleaseStatus(release, 'snoozed', snoozeMinutes)">{{ t('release.snooze') }}</button>
        <button class="btn-sm btn-danger-soft" v-if="isUnreadStatus(release.notification_status)" :disabled="isUpdating" @click="updateReleaseStatus(release, 'ignored')">{{ t('release.ignore') }}</button>
        <button class="btn-icon-link release-link-action" :disabled="isUpdating" @click="handleGoRelease(release)" @contextmenu.prevent.stop="releaseContextMenu($event, release.html_url)" :title="t('release.open_link')">
          <svg><use href="/icons.svg#link-icon"/></svg>
        </button>
        <span class="release-date">{{ t('release.published_at', formatDate(release.published_at)) }}</span>
      </div>
    </div>
    <!-- YouTube：B 站风格（左封面 + 右标题/简介），阅读全文进详情弹窗 -->
    <div v-if="isYoutube" class="yt-layout">
      <button
        class="yt-thumb-btn"
        :disabled="isUpdating"
        :title="t('release.open_link')"
        @click="handleGoRelease(release)"
        @contextmenu.prevent.stop="releaseContextMenu($event, release.html_url)"
      >
        <img v-if="youtubeThumb" class="yt-thumb" :src="youtubeThumb" alt="" loading="lazy" referrerpolicy="no-referrer" />
        <span v-if="youtubeDuration" class="yt-duration-badge">{{ youtubeDuration }}</span>
        <span v-if="youtubeIsLive" class="yt-live-badge">{{ t('release.yt_live') }}</span>
      </button>
      <div class="yt-info">
        <div v-if="releaseDisplayTitle(release)" class="release-title release-title-yt">{{ releaseDisplayTitle(release) }}</div>
        <div
          v-if="previewContent"
          class="yt-desc"
          :title="t('release.read_full')"
          @click="openDetail"
          @contextmenu.prevent.stop="handleSummaryContextMenu($event, previewContent)"
        >
          <MarkdownContent :content="previewContent" />
        </div>
        <!-- 底部行：播放量（左）+ 阅读全文（右），复用按钮行不额外占高 -->
        <div v-if="youtubeViewText || canOpenDetail" class="yt-footer-row">
          <span v-if="youtubeViewText" class="yt-view-count" :title="youtubeViewTitle">
            <svg><use href="/icons.svg#play-icon"/></svg>{{ youtubeViewText }}
          </span>
          <button v-if="canOpenDetail" class="btn-sm release-expand-btn" @click="openDetail">
            {{ t('release.read_full') }}
          </button>
        </div>
      </div>
    </div>
    <!-- 其它源：标题 + 摘要/原文预览 -->
    <template v-else>
      <div v-if="releaseDisplayTitle(release)" class="release-title">{{ releaseDisplayTitle(release) }}</div>
      <div v-if="previewContent" class="release-content">
        <!-- 摘要：3 行 clamp + 悬浮提示；前两行满行，阅读全文按钮悬浮第三行右侧（不足两行时独立成行） -->
        <div v-if="previewKind === 'summary'" class="release-summary-line" :class="{ 'has-third-line': summaryHasThirdLine }">
          <span
            ref="summaryTextRef"
            class="release-summary-text"
            tabindex="0"
            @mouseenter="handleSummaryEnter($event, previewContent)"
            @mousemove="handleSummaryMove"
            @mouseleave="hideSummaryTooltip"
            @focus="handleSummaryFocus($event, previewContent)"
            @blur="hideSummaryTooltip"
            @contextmenu.prevent.stop="handleSummaryContextMenu($event, previewContent)"
          >{{ summaryDisplay ?? previewContent }}</span>
          <button v-if="canOpenDetail" class="btn-sm release-expand-btn" @click="openDetail">
            {{ t('release.read_full') }}
          </button>
        </div>
        <!-- 译文 / 原文：Markdown 截断预览，点击打开详情弹窗阅读全文 -->
        <div
          v-else
          class="release-body-text"
          :title="t('release.read_full')"
          @click="openDetail"
          @contextmenu.prevent.stop="handleSummaryContextMenu($event, previewContent)"
        >
          <MarkdownContent :content="previewContent" />
        </div>
        <!-- 翻译中提示：内容保持显示，不打断阅读 -->
        <div v-if="translating" class="release-translating-hint">
          {{ t('release.translating_hint') }}
        </div>
        <button v-if="canOpenDetail && previewKind !== 'summary'" class="btn-sm release-expand-btn" @click="openDetail">
          {{ t('release.read_full') }}
        </button>
      </div>
    </template>
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

/* YouTube 视频标题：14px 半粗（与版本号 tag 同重量级），长标题最多两行截断 */
.release-title-yt {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  line-height: 1.5;
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  width: 100%;
}

.yt-view-count {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-muted);
  white-space: nowrap;
  cursor: default;
}

.yt-view-count svg {
  width: 11px;
  height: 11px;
}

/* 底部行：播放量（左）+ 阅读全文（右），复用按钮行不额外占高 */
.yt-footer-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin-top: 4px;
}

/* 阅读全文按钮恒定右对齐（无播放量时也保持右侧，不再回退左对齐） */
.yt-footer-row .release-expand-btn {
  margin-top: 0;
  margin-left: auto;
  flex-shrink: 0;
}

.release-repo-yt {
  color: var(--text-muted);
  font-weight: 500;
}

/* YouTube B 站风格布局：左封面 + 右标题/简介 */
.yt-layout {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-top: 6px;
  min-width: 0;
}

.yt-thumb-btn {
  position: relative;
  flex-shrink: 0;
  width: 200px;
  max-width: 42%;
  padding: 0;
  border: none;
  background: none;
  cursor: pointer;
  line-height: 0;
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.yt-thumb-btn:disabled {
  cursor: default;
}

.yt-thumb {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-subtle);
}

.yt-thumb-btn:hover .yt-thumb {
  border-color: var(--border-strong);
}

.yt-live-badge {
  position: absolute;
  right: 6px;
  bottom: 6px;
  padding: 1px 6px;
  border-radius: var(--radius-xs);
  background: #ff0000;
  color: #ffffff;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
  line-height: 14px;
}

/* 视频时长角标：左下角（B 站风格），仅 Data API 模式有 duration 时显示 */
.yt-duration-badge {
  position: absolute;
  left: 6px;
  bottom: 6px;
  padding: 1px 5px;
  border-radius: var(--radius-xs);
  background: rgba(0, 0, 0, 0.75);
  color: #ffffff;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.02em;
  line-height: 14px;
}

.yt-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  /* 子元素收缩到内容宽：阅读全文按钮不再被 stretch 拉满整行 */
  align-items: flex-start;
}

.yt-desc {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
  cursor: pointer;
  border-radius: 4px;
  width: 100%;
}

.yt-desc:hover {
  color: var(--text);
  background: var(--bg-subtle);
}

.release-date {
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}


.release-tag-hf {
  cursor: help;
  text-decoration: underline dotted var(--primary-soft-border);
}

.release-hf-tooltip {
  white-space: nowrap;
}

/* 摘要行：文字 3 行截断；前两行满行，阅读全文按钮悬浮第三行右侧（不足两行时独立成行） */
.release-summary-line {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  margin-top: 4px;
  color: var(--text);
  line-height: 1.55;
}

/* 内容左对齐、阅读全文按钮右对齐（与视频卡片统一） */
.release-content {
  display: flex;
  flex-direction: column;
  margin-top: 4px;
}

.release-content .release-expand-btn {
  align-self: flex-end;
}

.release-body-text {
  margin-top: 4px;
  color: var(--text);
  font-size: 13px;
  line-height: 1.6;
  max-height: 9em;
  overflow: hidden;
  border-radius: 4px;
  cursor: pointer;
}

.release-body-text:hover {
  background: var(--bg-subtle);
}

.release-expand-btn {
  margin-top: 4px;
  font-size: 11px;
}

/* 摘要行内按钮：默认独立成行右对齐 */
.release-summary-line .release-expand-btn {
  margin-top: 4px;
  flex-shrink: 0;
}

/* 摘要超过两行时：按钮悬浮在第三行右侧（盖住行尾，视觉上第三行在按钮处截断） */
.release-summary-line.has-third-line .release-expand-btn {
  position: absolute;
  right: 0;
  /* 按钮高（约 28px）大于行高（约 20px）：下移半个差值使其相对第三行垂直居中，
     上缘只进入上一行的行盒留白，不与文字重叠 */
  bottom: -4px;
  margin-top: 0;
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

.release-summary-text {
  min-width: 0;
  width: 100%;
  display: -webkit-box;
  -webkit-line-clamp: 3;
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

.release-status-meta {
  font-size: 12px;
  color: var(--text-muted);
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
