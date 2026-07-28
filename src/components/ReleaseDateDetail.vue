<script setup lang="ts">
import { computed } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { getLocale, t } from '../i18n'
import { parseDateKey, toDateKey } from '../utils/dateKey'
import ReleaseItem from './ReleaseItem.vue'

const props = defineProps<{
  selectedDate: string
  releases: ReleaseInfo[]
}>()

const emit = defineEmits<{
  update: []
  back: []
  // open-detail 携带导航序列：日历视图使用当日版本序列
  'open-detail': [release: ReleaseInfo, sequence: ReleaseInfo[]]
}>()

const dateDetailReleases = computed(() => {
  return props.releases
    .filter(release => toDateKey(new Date(release.published_at)) === props.selectedDate)
    .sort((a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime())
})

const dateDetailTitle = computed(() => {
  const d = parseDateKey(props.selectedDate)
  const locale = getLocale()
  return d.toLocaleDateString(locale, { year: 'numeric', month: 'long', day: 'numeric' })
})
</script>

<template>
  <button class="calendar-back" @click="emit('back')">
    <svg><use href="/icons.svg#chevron-left-icon"/></svg>
    {{ t('release.back_calendar') }}
  </button>
  <div class="date-detail-title">{{ dateDetailTitle }}</div>
  <div class="release-list">
    <div v-if="dateDetailReleases.length === 0" class="empty">{{ t('release.no_match') }}</div>
    <ReleaseItem
      v-for="release in dateDetailReleases"
      :key="release.id"
      :release="release"
      @update="emit('update')"
      @open-detail="(release) => emit('open-detail', release, dateDetailReleases)"
    />
  </div>
</template>

<style scoped>
.release-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.calendar-back {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border: none;
  background: var(--bg-subtle);
  color: var(--text);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
  margin-bottom: 12px;
  transition: background 0.1s;
}

.calendar-back:hover {
  background: var(--bg-hover);
}

.calendar-back svg {
  width: 14px;
  height: 14px;
}

.date-detail-title {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 12px;
  padding-left: 4px;
}
</style>
