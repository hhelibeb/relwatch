<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import type { ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import EmptyState from './common/EmptyState.vue'
import ReleaseItem from './ReleaseItem.vue'
import SkeletonList from './common/SkeletonList.vue'
import VirtualList from './common/VirtualList.vue'

const props = defineProps<{
  releases: ReleaseInfo[]
  isFiltering: boolean
  hasSearchQuery: boolean
  deepSearch: boolean
  /** 目录加载中：仅列表为空时渲染骨架，避免刷新已有列表时重放 */
  loading?: boolean
}>()

// open-detail 携带导航序列：简单视图使用全局时间倒序序列
// enable-deep：空结果提示「试试深度搜索」的点击出口（由 ReleaseTab 接线开启深度搜索）
// clear-search / clear-filters：空态下的两个出口，与空态成因对应
const emit = defineEmits<{
  update: []
  'open-detail': [release: ReleaseInfo, sequence: ReleaseInfo[]]
  'enable-deep': []
  'clear-search': []
  'clear-filters': []
}>()

// 空态分三级：因成因不同，文案与出口也不同（首次为空 / 搜索无结果 / 筛选无结果）
const emptyTitle = computed(() =>
  props.hasSearchQuery || props.isFiltering ? t('release.no_match') : t('release.empty'),
)
const emptyDescription = computed(() => {
  if (props.hasSearchQuery) {
    return props.deepSearch
      ? t('release.no_match_desc')
      : t('release.no_match_deep_hint', t('release.deep_search_label'))
  }
  return props.isFiltering ? t('release.no_match_desc') : t('release.empty_desc')
})
// 首次为空时不给清空类出口（没有可清的东西），也不留一个空的动作区
const hasEmptyAction = computed(() => props.hasSearchQuery || props.isFiltering)

const sortedReleases = computed(() => {
  return [...props.releases].sort(
    (a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime()
  )
})

// VirtualList 是泛型组件，InstanceType 不可用；这里只需要它的 scrollToIndex 能力
interface VirtualListHandle {
  scrollToIndex: (index: number) => void
}
const virtualList = ref<VirtualListHandle | null>(null)
// 通知定位高亮：目标 id 短暂高亮后自动清除（由 VirtualList 滚到可视区后设置）
const highlightId = ref<number | null>(null)
let highlightTimer: ReturnType<typeof setTimeout> | null = null

// 卸载时清掉挂起的高亮定时器，避免对已卸载实例继续写状态
onUnmounted(() => {
  if (highlightTimer) clearTimeout(highlightTimer)
  highlightTimer = null
})

/** 定位到指定 release：滚动使其可见并短暂高亮。返回是否找到。 */
function focusReleaseId(id: number): boolean {
  const index = sortedReleases.value.findIndex(r => r.id === id)
  if (index < 0) return false
  // 先滚动到目标（虚拟列表会确保对应行渲染），滚动完成后再置高亮并等待元素挂载
  virtualList.value?.scrollToIndex(index)
  const clearTimer = () => {
    if (highlightTimer) clearTimeout(highlightTimer)
    highlightTimer = null
  }
  clearTimer()
  highlightId.value = id
  highlightTimer = setTimeout(() => {
    highlightId.value = null
    highlightTimer = null
  }, 2600)
  return true
}

defineExpose({ focusReleaseId })
</script>

<template>
  <div class="release-list">
    <SkeletonList
      v-if="props.loading && sortedReleases.length === 0"
      variant="card"
      :rows="2"
      :label="t('common.loading')"
    />
    <EmptyState
      v-else-if="sortedReleases.length === 0"
      :title="emptyTitle"
      :description="emptyDescription"
    >
      <template v-if="hasEmptyAction" #actions>
        <button
          v-if="props.hasSearchQuery && !props.deepSearch"
          type="button"
          class="btn-sm"
          @click="emit('enable-deep')"
        >{{ t('release.deep_search_label') }}</button>
        <button
          v-if="props.hasSearchQuery"
          type="button"
          class="btn-sm"
          @click="emit('clear-search')"
        >{{ t('common.clear_search') }}</button>
        <button
          v-else-if="props.isFiltering"
          type="button"
          class="btn-sm"
          @click="emit('clear-filters')"
        >{{ t('release.filter_clear_all') }}</button>
      </template>
    </EmptyState>
    <VirtualList
      v-else
      ref="virtualList"
      :items="sortedReleases"
      :item-key="release => release.id"
    >
      <template #default="{ item }">
        <ReleaseItem
          :release="item"
          :highlighted="highlightId === item.id"
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
