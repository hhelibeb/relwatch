<script setup lang="ts">
import ReleaseSearchBar from './ReleaseSearchBar.vue'
import type { ReleaseImportanceFilter, ReleaseSourceFilter, ReleaseStatusFilter, ViewMode } from './releaseTypes'

// 兼容旧的 ReleaseSearchBar 公共接口，同时把 ReleaseTab 的工具栏职责命名为独立组件。
const props = defineProps<{
  modelValue: string
  statusFilter: ReleaseStatusFilter
  importanceFilter: ReleaseImportanceFilter
  sourceFilter: ReleaseSourceFilter
  viewMode: ViewMode
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
  'update:importanceFilter': [value: ReleaseImportanceFilter]
  'update:sourceFilter': [value: ReleaseSourceFilter]
  'update:viewMode': [value: ViewMode]
  searchEnter: []
}>()

function updateStatusFilter(value: ReleaseStatusFilter) {
  emit('update:statusFilter', value)
}

function updateImportanceFilter(value: ReleaseImportanceFilter) {
  emit('update:importanceFilter', value)
}

function updateSourceFilter(value: ReleaseSourceFilter) {
  emit('update:sourceFilter', value)
}

function updateViewMode(value: ViewMode) {
  emit('update:viewMode', value)
}
</script>

<template>
  <ReleaseSearchBar
    :model-value="props.modelValue"
    :status-filter="props.statusFilter"
    :importance-filter="props.importanceFilter"
    :source-filter="props.sourceFilter"
    :view-mode="props.viewMode"
    :show-search="true"
    @update:model-value="emit('update:modelValue', $event)"
    @update:status-filter="updateStatusFilter"
    @update:importance-filter="updateImportanceFilter"
    @update:source-filter="updateSourceFilter"
    @update:view-mode="updateViewMode"
    @search-enter="emit('searchEnter')"
  />
</template>
