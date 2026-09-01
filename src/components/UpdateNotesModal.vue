<script setup lang="ts">
// 应用内更新：新版本 Release Note 弹窗。
//
// 为什么不复用 ReleaseDetailModal：那个组件的 props 是 ReleaseInfo，内部深度耦合
// 了翻译（调 translateRelease(release.id)）、上下版导航、通知状态徽标、拖拽 resize
// 持久化、右键菜单等业务。更新检查拿到的只是 {version, date, body} 三元组，
// 喂一个假 ReleaseInfo（id 用 -1）会触发一堆分支——「翻译」按钮由 canTranslateRelease
// 只看 aiEnabled 判定，点下去会拿假 id 打后端；导航按钮要逐个 v-if 关掉。纯靠补丁
// 反而把组件搞脏且容易漏。这里只复用 MarkdownContent（props 仅 content，
// marked + DOMPurify 清洗，无业务耦合），其余自绘约 100 行。
import { ref, onMounted, onUnmounted, computed } from 'vue'
import MarkdownContent from './common/MarkdownContent.vue'
import { openReleaseUrl } from '../api/client'
import { registerOverlayActive } from '../composables/contextMenuBus'
import { track } from '../composables/useUsageTracking'
import { t } from '../i18n'
import { formatDate } from '../utils'

const props = defineProps<{
  version: string
  /** RFC3339 字符串（latest.json 的 pub_date），可能为空 */
  date: string | null
  /** latest.json 的 notes（Markdown），可能为空 */
  body: string | null
}>()

const emit = defineEmits<{ close: [] }>()

const modalEl = ref<HTMLElement | null>(null)
let unregisterOverlay: (() => void) | null = null

/** latest.json 的 pub_date 时刻尚未发布（CI 生成产物时写入），
 *  与 GitHub Release 的发布时间不是一回事，因此措辞用「构建」而非「发布」。 */
const dateText = computed(() => (props.date ? formatDate(props.date) : ''))

/** Release 页面地址：沿用 openReleaseNotes 的既有拼法（tags/v<version>） */
const releaseUrl = computed(() => `https://github.com/hhelibeb/relwatch/releases/tag/v${props.version}`)

function handleClose() {
  track('update.notes_close')
  emit('close')
}

function handleOpenInBrowser(): void {
  track('update.notes_open_browser')
  void openReleaseUrl(releaseUrl.value)
}

function handleKeydown(e: KeyboardEvent) {
  if (e.defaultPrevented) return
  if (e.key === 'Escape') handleClose()
}

onMounted(() => {
  window.addEventListener('keydown', handleKeydown)
  // 弹窗挂载即视为覆盖层打开：弹窗内 Esc 不应最小化到托盘（供 useEscapeToTray 判定）
  unregisterOverlay = registerOverlayActive(() => true)
})
onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown)
  unregisterOverlay?.()
})
</script>

<template>
  <Teleport to="body">
    <div class="update-notes-overlay" @click.self="handleClose">
      <div ref="modalEl" class="update-notes-modal" role="dialog" aria-modal="true">
        <div class="update-notes-header">
          <div class="update-notes-heading">
            <span class="update-notes-version">v{{ version }}</span>
            <span v-if="dateText" class="update-notes-date">{{ t('update.notes_built_at', dateText) }}</span>
          </div>
          <button class="update-notes-close" :title="t('release.detail_close')" @click="handleClose">
            <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>
          </button>
        </div>
        <div class="update-notes-body">
          <div v-if="body" class="update-notes-markdown">
            <MarkdownContent :content="body" />
          </div>
          <!-- 防御性空态：SettingsTab 挂载条件是 `showUpdateNotes && updateNotesBody`（body 非空），
               生产路径拿不到空 body；此处兜底仅在组件被独立复用（未来其他调用方或本文件的
               「body 为空时显示空态」用例）时生效，避免弹窗渲染空白框。 -->
          <div v-else class="update-notes-empty">{{ t('update.notes_empty') }}</div>
        </div>
        <div class="update-notes-footer">
          <!-- 保留浏览器入口：GitHub Release 页上有安装包、commit 与历史版本，
               是弹窗内 Markdown 覆盖不到的部分 -->
          <button class="btn-sm" @click="handleOpenInBrowser">{{ t('update.notes_open_in_browser') }}</button>
          <button class="btn-sm" @click="handleClose">{{ t('release.detail_close') }}</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
/* 层级与 ReleaseDetailModal 一致：高于列表 tooltip(10002) 与 ContextMenu(10000) */
.update-notes-overlay {
  position: fixed;
  inset: 0;
  z-index: 10010;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  padding: 24px;
}

.update-notes-modal {
  display: flex;
  flex-direction: column;
  width: min(620px, 100%);
  max-height: calc(100vh - 48px);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.update-notes-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 14px 16px 0;
}

.update-notes-heading {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.update-notes-version {
  font-weight: 600;
  font-size: 15px;
  color: var(--primary);
  flex-shrink: 0;
}

.update-notes-date {
  font-size: 12px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.update-notes-close {
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

.update-notes-close:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.update-notes-close svg {
  width: 14px;
  height: 14px;
}

.update-notes-body {
  flex: 1;
  min-height: 120px;
  overflow-y: auto;
  padding: 10px 16px 14px;
}

.update-notes-empty {
  padding: 24px 12px;
  color: var(--text-muted);
  font-size: 13px;
  text-align: center;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
}

.update-notes-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  padding: 10px 16px;
  border-top: 1px solid var(--border);
}
</style>
