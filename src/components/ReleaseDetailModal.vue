<script setup lang="ts">
import { ref, computed, watch, inject, nextTick, onMounted, onUnmounted } from 'vue'
import MarkdownContent from './common/MarkdownContent.vue'
import ContextMenu, { type ContextMenuItem } from './common/ContextMenu.vue'
import { ShowToastKey, AiEnabledKey } from '../injection-keys'
import { type ReleaseInfo } from '../api/releases'
import { openReleaseUrl, copyImageToClipboard, copyTextToClipboard } from '../api/client'
import { useDragResize, type ResizeDir } from '../composables/useDragResize'
import { registerCloser, unregisterCloser, closeAllContextMenus, registerOverlayActive } from '../composables/contextMenuBus'
import { track } from '../composables/useUsageTracking'
import { useReleaseTranslate } from '../composables/useReleaseTranslate'
import { t } from '../i18n'
import { formatDate, statusClass, statusLabel } from '../utils'
import { releaseDisplayTitle, releaseImportanceText, releaseImportanceClass, canTranslateRelease } from '../utils/releaseDisplay'
import type { ReleaseContentMode } from './releaseTypes'
import { getSourceTypeDef } from '../api/source-registry'

// 版本详情弹窗：卡片只展示单一预览（摘要 > 译文 > 原文），点击后进入弹窗完整阅读，
// 摘要/译文/原文的内容切换集中在弹窗内进行（卡片不再提供标签）。
const props = defineProps<{
  release: ReleaseInfo
  position: number // 1-based，在当前过滤序列中的位置
  total: number
  hasPrev: boolean
  hasNext: boolean
}>()

const emit = defineEmits<{
  close: []
  navigate: [delta: number] // -1 = 更新的版本，1 = 更旧的版本
  update: []
}>()

const showToast = inject(ShowToastKey, () => {})
const aiEnabledRef = inject(AiEnabledKey, ref(false))
const aiEnabled = computed(() => aiEnabledRef.value)

// ========== 拖动 / 调整大小 ==========
// 拖动标题栏移动弹窗，八向手柄调整大小；位置与显式尺寸持久化，下次打开保持
const modalEl = ref<HTMLElement | null>(null)
const { startDrag, startResize } = useDragResize(modalEl, {
  minWidth: 400,
  minHeight: 300,
  persistKey: 'relwatch.release-detail.rect',
})
const resizeDirs: ResizeDir[] = ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw']

// ========== 正文右键菜单 ==========
// 应用全局的 contextmenu 处理对弹窗无效（全局菜单层级低于弹窗 overlay，会被遮挡），
// 弹窗自行提供：链接 → 打开/复制链接；图片 → 复制图片/复制图片链接/打开；
// 有选区 → 复制；否则 → 复制内容。
type BodyMenuState = {
  x: number
  y: number
  kind: 'link' | 'image' | 'text'
  link?: string
  imgSrc?: string
  selection?: string
}
const bodyMenu = ref<BodyMenuState | null>(null)

const bodyMenuItems = computed<ContextMenuItem[]>(() => {
  const m = bodyMenu.value
  if (!m) return []
  if (m.kind === 'link') {
    return [
      { id: 'openLink', label: t('context.open') },
      { id: 'copyLink', label: t('context.copy_link') },
    ]
  }
  if (m.kind === 'image') {
    return [
      { id: 'copyImage', label: t('context.copy_image') },
      { id: 'copyImageLink', label: t('context.copy_image_link') },
      { id: 'openImage', label: t('context.open') },
    ]
  }
  return m.selection
    ? [{ id: 'copySelection', label: t('context.copy') }]
    : [{ id: 'copyContent', label: t('context.copy_content') }]
})

function closeBodyMenu() {
  bodyMenu.value = null
}

let unregisterOverlay: (() => void) | null = null

function handleClose() {
  track('release.detail_close')
  emit('close')
}

function handleBodyContextMenu(e: MouseEvent) {
  const target = e.target as HTMLElement
  closeAllContextMenus()
  const anchor = target.closest('a[href]') as HTMLAnchorElement | null
  if (anchor && /^https?:\/\//i.test(anchor.href)) {
    bodyMenu.value = { x: e.clientX, y: e.clientY, kind: 'link', link: anchor.href }
    return
  }
  const img = target.closest('img[src]') as HTMLImageElement | null
  if (img && /^https?:\/\//i.test(img.src)) {
    bodyMenu.value = { x: e.clientX, y: e.clientY, kind: 'image', imgSrc: img.src }
    return
  }
  const selection = window.getSelection()?.toString() ?? ''
  bodyMenu.value = { x: e.clientX, y: e.clientY, kind: 'text', selection }
}

async function copyText(text: string) {
  try {
    await copyTextToClipboard(text)
    showToast?.(t('release.copied'))
  } catch (e: unknown) {
    showToast?.(t('release.copy_failed') + (e instanceof Error ? e.message : String(e)))
  }
}

async function handleBodyMenuAction(actionId: string) {
  const m = bodyMenu.value
  if (!m) return
  bodyMenu.value = null
  switch (actionId) {
    case 'openLink':
      if (m.link) openReleaseUrl(m.link)
      break
    case 'copyLink':
      track('release.copy')
      if (m.link) await copyText(m.link)
      break
    case 'openImage':
      track('release.open')
      if (m.imgSrc) openReleaseUrl(m.imgSrc)
      break
    case 'copyImageLink':
      track('release.copy')
      if (m.imgSrc) await copyText(m.imgSrc)
      break
    case 'copyImage':
      track('release.copy')
      if (m.imgSrc) {
        try {
          await copyImageToClipboard(m.imgSrc)
          showToast?.(t('release.copied'))
        } catch (e: unknown) {
          showToast?.(t('release.copy_image_failed') + (e instanceof Error ? e.message : String(e)))
        }
      }
      break
    case 'copySelection':
      track('release.copy')
      if (m.selection) await copyText(m.selection)
      break
    case 'copyContent':
      await handleCopyContent()
      break
  }
}

// ========== 显示模式切换：摘要 / 译文 / 原文 ==========
// 弹窗承载全部内容切换。默认视图优先全文（译文 > 原文）：摘要在卡片上已经看过，
// 「阅读全文」的意图就是读完整内容；摘要标签保留供回看。
type ViewMode = ReleaseContentMode

function defaultViewMode(): ViewMode {
  if (props.release.body_translated) return 'translated'
  if (props.release.body) return 'full'
  return 'summary'
}

// 逐版本导航时解析期望的内容模式：目标版本有对应内容则采用，否则回退默认优先级
function resolveMode(mode: ViewMode | null | undefined): ViewMode {
  switch (mode) {
    case 'summary': return props.release.ai_summary ? 'summary' : defaultViewMode()
    case 'translated': return props.release.body_translated ? 'translated' : defaultViewMode()
    case 'full': return props.release.body ? 'full' : defaultViewMode()
    default: return defaultViewMode()
  }
}

const viewMode = ref<ViewMode>(defaultViewMode())
const bodyEl = ref<HTMLElement | null>(null)

// 翻译状态机（与 ReleaseItem 卡片共用同一实现）：
// - 开始时切到译文视图（显示「翻译中」占位）
// - 成功后 emit update 刷新列表；失败回退全文视图
// - body_translated 从无到有时自动切到译文视图（onTranslated）
const { translating, handleTranslateRelease } = useReleaseTranslate({
  release: () => props.release,
  showToast,
  onStart: () => { viewMode.value = 'translated' },
  onSuccess: () => emit('update'),
  onError: () => { viewMode.value = 'full' },
  onTranslated: () => {
    if (viewMode.value === 'full') {
      viewMode.value = 'translated'
    }
  },
})

const availableModes = computed<{ mode: ViewMode; label: string }[]>(() => {
  const modes: { mode: ViewMode; label: string }[] = []
  if (props.release.ai_summary) modes.push({ mode: 'summary', label: t('release.view_summary') })
  if (props.release.body_translated) {
    modes.push({ mode: 'translated', label: t('release.view_translated') })
  } else if (translating.value) {
    modes.push({ mode: 'translated', label: t('release.view_translating') })
  }
  if (props.release.body) modes.push({ mode: 'full', label: t('release.view_full') })
  return modes
})

const currentContent = computed<string | null>(() => {
  switch (viewMode.value) {
    case 'summary': return props.release.ai_summary
    case 'translated':
      if (translating.value && !props.release.body_translated) return null
      return props.release.body_translated
    case 'full': return props.release.body
    default: return null
  }
})

function switchMode(mode: ViewMode) {
  viewMode.value = mode
  track('release.detail_mode')
}

// 切换到另一个版本时：保持当前内容模式（便于逐版本对比译文/原文），
// 目标版本没有该内容时回退默认优先级；清除翻译中状态、内容区滚动回顶部
watch(() => props.release.id, () => {
  viewMode.value = resolveMode(viewMode.value)
  translating.value = false
  nextTick(() => bodyEl.value?.scrollTo({ top: 0 }))
})

// ========== 操作 ==========
// 弹窗仅在全文视图下允许翻译（卡片无视图概念，直接用基础条件）
const canTranslate = computed(() =>
  viewMode.value === 'full'
  && canTranslateRelease(props.release, aiEnabled.value)
)

async function handleCopyContent() {
  const content = currentContent.value
  if (!content) return
  track('release.copy')
  await copyText(content)
}

function handleOpenLink() {
  track('release.open')
  openReleaseUrl(props.release.html_url)
}

// ========== 键盘交互：Esc 关闭，←/→ 在版本间导航 ==========
function handleKeydown(e: KeyboardEvent) {
  // ContextMenu 的键盘导航/Esc 已 preventDefault，不重复处理（避免 Esc 连弹窗一起关）
  if (e.defaultPrevented) return
  if (e.key === 'Escape') {
    if (bodyMenu.value) {
      closeBodyMenu()
      return
    }
    handleClose()
  } else if (e.key === 'ArrowLeft' && props.hasPrev) {
    if (bodyMenu.value) return
    emit('navigate', -1)
  } else if (e.key === 'ArrowRight' && props.hasNext) {
    if (bodyMenu.value) return
    emit('navigate', 1)
  }
}

onMounted(() => {
  window.addEventListener('keydown', handleKeydown)
  registerCloser(closeBodyMenu)
  document.addEventListener('click', closeBodyMenu)
  // 弹窗挂载即视为覆盖层打开：弹窗内 Esc 不应最小化到托盘（供 useEscapeToTray 判定）
  unregisterOverlay = registerOverlayActive(() => true)
})
onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown)
  unregisterCloser(closeBodyMenu)
  document.removeEventListener('click', closeBodyMenu)
  unregisterOverlay?.()
})

// HF 源 tag_name 已含组织名，不重复显示 owner/repo 前缀（注册表能力声明）
const showReleaseRepo = computed(() => getSourceTypeDef(props.release.source_type)?.showRepoInDetail !== false)
</script>

<template>
  <Teleport to="body">
    <div class="release-detail-overlay" @click.self="handleClose">
      <div ref="modalEl" class="release-detail-modal" role="dialog" aria-modal="true">
        <div class="release-detail-header" @pointerdown="startDrag">
          <div class="release-detail-heading">
            <span v-if="showReleaseRepo" class="release-detail-repo">{{ release.owner }}/{{ release.repo }}</span>
            <span class="release-detail-tag">{{ release.tag_name }}</span>
            <span v-if="releaseImportanceText(release)" class="release-importance-chip" :class="releaseImportanceClass(release)">{{ releaseImportanceText(release) }}</span>
            <span v-if="release.prerelease" class="pre-release-badge">{{ t('release.prerelease') }}</span>
            <span class="status-inline" :class="statusClass(release.notification_status, release.snooze_until)">{{ statusLabel(release.notification_status, release.snooze_until) }}</span>
          </div>
          <button class="release-detail-close" :title="t('release.detail_close')" @click="handleClose">
            <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>
          </button>
        </div>
        <div class="release-detail-meta">
          <span class="release-detail-date">{{ t('release.published_at', formatDate(release.published_at)) }}</span>
        </div>
        <div v-if="releaseDisplayTitle(release)" class="release-detail-title">{{ releaseDisplayTitle(release) }}</div>
        <div v-if="availableModes.length > 1" class="release-view-tabs release-detail-tabs">
          <button
            v-for="m in availableModes"
            :key="m.mode"
            class="release-view-tab"
            :class="{ active: viewMode === m.mode }"
            @click="switchMode(m.mode)"
          >{{ m.label }}</button>
        </div>
        <div ref="bodyEl" class="release-detail-body" @contextmenu.prevent.stop="handleBodyContextMenu">
          <!-- 译文 / 原文：完整 Markdown 渲染，无高度限制 -->
          <div v-if="currentContent" class="release-detail-markdown">
            <MarkdownContent :content="currentContent" />
          </div>
          <!-- 翻译中占位 -->
          <div v-else-if="translating && viewMode === 'translated'" class="release-detail-translating">
            {{ t('release.translating_hint') }}
          </div>
          <div v-else class="release-detail-translating">{{ t('release.detail_empty') }}</div>
        </div>
        <div class="release-detail-footer">
          <div class="release-detail-nav">
            <button class="btn-sm" :disabled="!hasPrev" :title="t('release.prev_release')" @click="emit('navigate', -1)">
              <svg class="nav-icon"><use href="/icons.svg#chevron-left-icon"/></svg>
              {{ t('release.prev_release') }}
            </button>
            <span class="release-detail-position">{{ position }} / {{ total }}</span>
            <button class="btn-sm" :disabled="!hasNext" :title="t('release.next_release')" @click="emit('navigate', 1)">
              {{ t('release.next_release') }}
              <svg class="nav-icon"><use href="/icons.svg#chevron-right-icon"/></svg>
            </button>
          </div>
          <div class="release-detail-actions">
            <button v-if="canTranslate" class="btn-sm" :disabled="translating" @click="handleTranslateRelease">{{ t('context.translate') }}</button>
            <button class="btn-sm" :disabled="!currentContent" @click="handleCopyContent">{{ t('context.copy_content') }}</button>
            <button class="btn-sm" @click="handleOpenLink">{{ t('release.open_link') }}</button>
          </div>
        </div>
        <div
          v-for="dir in resizeDirs"
          :key="dir"
          class="resize-handle"
          :class="`resize-handle-${dir}`"
          @pointerdown="startResize($event, dir)"
        ></div>
      </div>
      <!-- 右键菜单必须放在 overlay 内、modal 外：modal 拖动后带 transform，
           会导致其内部 fixed 定位相对 modal 解析并被 overflow:hidden 裁剪 -->
      <ContextMenu
        v-if="bodyMenu"
        :x="bodyMenu.x"
        :y="bodyMenu.y"
        :items="bodyMenuItems"
        @action="handleBodyMenuAction"
        @close="closeBodyMenu"
      />
    </div>
  </Teleport>
</template>

<style scoped>
.release-detail-overlay {
  position: fixed;
  inset: 0;
  z-index: 10010; /* 高于列表 tooltip(10002) 与 ContextMenu(10000) */
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  padding: 24px;
}

.release-detail-modal {
  position: relative; /* resize 手柄的定位基座 */
  display: flex;
  flex-direction: column;
  width: min(760px, 100%);
  max-height: calc(100vh - 48px);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

/* 八向 resize 手柄：透明的边缘/角落热区，右下角带可见握把 */
.resize-handle {
  position: absolute;
  z-index: 2;
  touch-action: none;
}

.resize-handle-n {
  top: 0;
  left: 12px;
  right: 12px;
  height: 5px;
  cursor: ns-resize;
}

.resize-handle-s {
  bottom: 0;
  left: 12px;
  right: 12px;
  height: 5px;
  cursor: ns-resize;
}

.resize-handle-e {
  top: 12px;
  bottom: 12px;
  right: 0;
  width: 4px;
  cursor: ew-resize;
}

.resize-handle-w {
  top: 12px;
  bottom: 12px;
  left: 0;
  width: 4px;
  cursor: ew-resize;
}

.resize-handle-ne {
  top: 0;
  right: 0;
  width: 12px;
  height: 12px;
  cursor: nesw-resize;
}

.resize-handle-nw {
  top: 0;
  left: 0;
  width: 12px;
  height: 12px;
  cursor: nwse-resize;
}

.resize-handle-sw {
  bottom: 0;
  left: 0;
  width: 12px;
  height: 12px;
  cursor: nesw-resize;
}

.resize-handle-se {
  bottom: 0;
  right: 0;
  width: 14px;
  height: 14px;
  cursor: nwse-resize;
}

.resize-handle-se::after {
  content: '';
  position: absolute;
  inset: 3px;
  background: repeating-linear-gradient(-45deg, var(--text-faint) 0 1px, transparent 1px 4px);
  clip-path: polygon(100% 0, 100% 100%, 0 100%);
  opacity: 0.6;
}

.resize-handle-se:hover::after {
  opacity: 1;
}

.release-detail-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 14px 16px 0;
  cursor: move;
  touch-action: none;
}

.release-detail-heading {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.release-detail-repo {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.release-detail-tag {
  font-weight: 600;
  font-size: 15px;
  color: var(--primary);
  flex-shrink: 0;
}

.release-detail-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  flex-shrink: 0;
}

.release-detail-close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.release-detail-close svg {
  width: 14px;
  height: 14px;
}

.release-detail-meta {
  padding: 4px 16px 0;
}

.release-detail-date {
  font-size: 12px;
  color: var(--text-muted);
}

.release-detail-title {
  padding: 4px 16px 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.release-detail-tabs {
  margin: 8px 16px 0;
}

.release-detail-body {
  flex: 1;
  min-height: 120px;
  overflow-y: auto;
  padding: 10px 16px 14px;
}

.release-detail-markdown {
  color: var(--text);
  font-size: 13px;
  line-height: 1.6;
}

.release-detail-translating {
  padding: 24px 12px;
  color: var(--text-muted);
  font-size: 13px;
  text-align: center;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
}

.release-detail-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 16px;
  border-top: 1px solid var(--border);
  flex-wrap: wrap;
}

.release-detail-nav {
  display: flex;
  align-items: center;
  gap: 8px;
}

.release-detail-nav .btn-sm {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.nav-icon {
  width: 12px;
  height: 12px;
}

.release-detail-position {
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}

.release-detail-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.release-view-tabs {
  display: inline-flex;
  gap: 2px;
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
</style>
