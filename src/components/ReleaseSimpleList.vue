<script setup lang="ts">
import { computed } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import ReleaseItem from './ReleaseItem.vue'
import VirtualList from './common/VirtualList.vue'

const props = defineProps<{
  releases: ReleaseInfo[]
  isFiltering: boolean
}>()

// open-detail 携带导航序列：简单视图使用全局时间倒序序列
const emit = defineEmits<{ update: []; 'open-detail': [release: ReleaseInfo, sequence: ReleaseInfo[]] }>()

const sortedReleases = computed(() => {
  return [...props.releases].sort(
    (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime()
  )
})
</script>

<template>
  <div class="release-list">
    <div v-if="sortedReleases.length === 0" class="empty">
      {{ props.isFiltering ? t('release.no_match') : t('release.empty') }}
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
</style>
