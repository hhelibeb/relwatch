<script setup lang="ts">
import type { ComponentPublicInstance } from 'vue'
import MarkdownContent from '../common/MarkdownContent.vue'
import { t } from '../../i18n'

defineProps<{
  cardTitle: string
  previewKind: 'summary' | 'body' | null
  previewContent: string | null
  summaryDisplay: string | null
  summaryHasThirdLine: boolean
  translating: boolean
  canOpenDetail: boolean
  bindSummaryTextEl: (el: Element | ComponentPublicInstance | null) => void
}>()

const emit = defineEmits<{
  summaryEnter: [e: MouseEvent, content: string | null]
  summaryMove: [e: MouseEvent]
  summaryLeave: []
  summaryFocus: [e: FocusEvent, content: string | null]
  summaryBlur: []
  summaryContextMenu: [e: MouseEvent, content: string | null]
  openDetail: []
}>()
</script>

<template>
  <div v-if="cardTitle" class="release-title">{{ cardTitle }}</div>
  <div v-if="previewContent" class="release-content">
    <div v-if="previewKind === 'summary'" class="release-summary-line" :class="{ 'has-third-line': summaryHasThirdLine }">
      <span
        :ref="bindSummaryTextEl"
        class="release-summary-text"
        tabindex="0"
        @mouseenter="emit('summaryEnter', $event, previewContent)"
        @mousemove="emit('summaryMove', $event)"
        @mouseleave="emit('summaryLeave')"
        @focus="emit('summaryFocus', $event, previewContent)"
        @blur="emit('summaryBlur')"
        @contextmenu.prevent.stop="emit('summaryContextMenu', $event, previewContent)"
      >{{ summaryDisplay ?? previewContent }}</span>
      <button v-if="canOpenDetail" class="btn-sm release-expand-btn" @click="emit('openDetail')">
        {{ t('release.read_full') }}
      </button>
    </div>
    <div
      v-else
      class="release-body-text"
      :title="t('release.read_full')"
      @click="emit('openDetail')"
      @contextmenu.prevent.stop="emit('summaryContextMenu', $event, previewContent)"
    >
      <MarkdownContent :content="previewContent" />
    </div>
    <div v-if="translating" class="release-translating-hint">
      {{ t('release.translating_hint') }}
    </div>
    <button v-if="canOpenDetail && previewKind !== 'summary'" class="btn-sm release-expand-btn" @click="emit('openDetail')">
      {{ t('release.read_full') }}
    </button>
  </div>
</template>

<style scoped>
.release-title {
  font-size: var(--fs-body);
  color: var(--text);
  margin: 2px 0 4px;
}

.release-summary-line {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  margin-top: 4px;
  color: var(--text);
  line-height: 1.55;
}

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
  font-size: var(--fs-body);
  line-height: 1.6;
  max-height: 9em;
  overflow: hidden;
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.release-body-text:hover {
  background: var(--hover-surface-nested);
}

.release-expand-btn {
  margin-top: 4px;
  font-size: var(--fs-caption);
}

.release-expand-btn:hover {
  background: var(--hover-surface-nested);
}

.release-summary-line .release-expand-btn {
  margin-top: 4px;
  flex-shrink: 0;
}

.release-summary-line.has-third-line .release-expand-btn {
  position: absolute;
  right: 0;
  bottom: -4px;
  margin-top: 0;
}

.release-translating-hint {
  margin-top: 4px;
  padding: 12px;
  color: var(--text-muted);
  font-size: var(--fs-body);
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
  border-radius: var(--radius-sm);
}

.release-summary-text:focus-visible {
  outline: 2px solid var(--primary-soft-border);
  outline-offset: 2px;
}
</style>
