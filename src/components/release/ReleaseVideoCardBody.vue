<script setup lang="ts">
import MarkdownContent from '../common/MarkdownContent.vue'
import { t } from '../../i18n'

defineProps<{
  title: string | null
  previewContent: string | null
  thumbSrc: string
  duration: string | null
  isLive: boolean
  viewText: string | null
  viewTitle: string
  canOpenDetail: boolean
  isUpdating: boolean
}>()

const emit = defineEmits<{
  openLink: []
  coverContextMenu: [e: MouseEvent]
  openDetail: []
  summaryContextMenu: [e: MouseEvent, content: string | null]
}>()
</script>

<template>
  <div class="yt-layout">
    <button
      class="yt-thumb-btn"
      :disabled="isUpdating"
      :title="t('release.open_link')"
      @click="emit('openLink')"
      @contextmenu.prevent.stop="emit('coverContextMenu', $event)"
    >
      <img v-if="thumbSrc" class="yt-thumb" :src="thumbSrc" alt="" loading="lazy" referrerpolicy="no-referrer" />
      <span v-if="duration" class="yt-duration-badge">{{ duration }}</span>
      <span v-if="isLive" class="yt-live-badge">{{ t('release.yt_live') }}</span>
    </button>
    <div class="yt-info">
      <div v-if="title" class="release-title release-title-yt">{{ title }}</div>
      <div
        v-if="previewContent"
        class="yt-desc"
        :title="t('release.read_full')"
        @click="emit('openDetail')"
        @contextmenu.prevent.stop="emit('summaryContextMenu', $event, previewContent)"
      >
        <MarkdownContent :content="previewContent" />
      </div>
      <div v-if="viewText || canOpenDetail" class="yt-footer-row">
        <span v-if="viewText" class="yt-view-count" :title="viewTitle">
          <svg><use href="/icons.svg#play-icon" /></svg>{{ viewText }}
        </span>
        <button v-if="canOpenDetail" class="btn-sm release-expand-btn" @click="emit('openDetail')">
          {{ t('release.read_full') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
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
  font-size: var(--fs-caption);
  font-weight: 700;
  letter-spacing: 0.04em;
  line-height: 14px;
}

.yt-duration-badge {
  position: absolute;
  left: 6px;
  bottom: 6px;
  padding: 1px 5px;
  border-radius: var(--radius-xs);
  background: rgba(0, 0, 0, 0.75);
  color: #ffffff;
  font-size: var(--fs-caption);
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
  align-items: flex-start;
}

.release-title-yt {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  line-height: 1.5;
  margin: 0;
  font-size: var(--fs-title);
  font-weight: 600;
  width: 100%;
}

.yt-desc {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-meta);
  line-height: 1.6;
  cursor: pointer;
  border-radius: var(--radius-sm);
  width: 100%;
}

.yt-desc:hover {
  color: var(--text);
  background: var(--hover-surface-nested);
}

.yt-view-count {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
  font-size: var(--fs-meta);
  font-weight: 500;
  color: var(--text-muted);
  white-space: nowrap;
  cursor: default;
}

.yt-view-count svg {
  width: 11px;
  height: 11px;
}

.yt-footer-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  margin-top: 4px;
}

.yt-footer-row .release-expand-btn {
  margin-top: 0;
  margin-left: auto;
  flex-shrink: 0;
  font-size: var(--fs-caption);
}

.release-expand-btn:hover {
  background: var(--hover-surface-nested);
}
</style>
