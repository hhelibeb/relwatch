<script setup lang="ts">
import { computed } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import ReleaseItem from './ReleaseItem.vue'
import VirtualList from './common/VirtualList.vue'

const props = defineProps<{
  releases: ReleaseInfo[]
  isFiltering: boolean
  hasSearchQuery: boolean
  deepSearch: boolean
}>()

// open-detail 携带导航序列：简单视图使用全局时间倒序序列
// enable-deep：空结果提示「试试深度搜索」的点击出口（由 ReleaseTab 接线开启深度搜索）
const emit = defineEmits<{ update: []; 'open-detail': [release: ReleaseInfo, sequence: ReleaseInfo[]]; 'enable-deep': [] }>()

const sortedReleases = computed(() => {
  return [...props.releases].sort(
    (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime()
  )
})
</script>

<template>
  <div class="release-list">
    <div v-if="sortedReleases.length === 0" class="empty">
      <template v-if="props.hasSearchQuery">
        <!-- 有搜索词：深度搜索未开启时给出一键开启的出口，开启后显示普通无匹配 -->
        <button
          v-if="!props.deepSearch"
          type="button"
          class="empty-deep-hint"
          @click="emit('enable-deep')"
        >{{ t('release.no_match_deep_hint', t('release.deep_search_label')) }}</button>
        <template v-else>{{ t('release.no_match') }}</template>
      </template>
      <template v-else>
        {{ props.isFiltering ? t('release.no_match') : t('release.empty') }}
      </template>
    </div>
    <VirtualList
      v-else
      :items="sortedReleases"
      :item-key="release => release.id"
    >
      <template #default="{ item }">
        <ReleaseItem
          :release="item"
          @update="emit('update')"
          @open-detail="(release) => emit('open-detail', release, sortedReleases)"
        />
      </template>
    </VirtualList>
  </div>
</template>

<style scoped>
.release-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* 空结果里的「试试深度搜索」提示：整句可点，链接风格 */
.empty-deep-hint {
  border: none;
  background: transparent;
  padding: 0;
  color: var(--primary);
  font-size: inherit;
  font-family: inherit;
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 3px;
}

.empty-deep-hint:hover {
  color: var(--primary-hover);
}
</style>
