<script setup lang="ts">
import { computed } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import ReleaseItem from './ReleaseItem.vue'

const props = defineProps<{
  releases: ReleaseInfo[]
  isFiltering: boolean
}>()

const emit = defineEmits<{ update: [] }>()

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
    <ReleaseItem
      v-for="release in sortedReleases"
      :key="release.id"
      :release="release"
      @update="emit('update')"
    />
  </div>
</template>

<style scoped>
.release-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
</style>
