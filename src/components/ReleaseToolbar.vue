<script setup lang="ts">
import ReleaseSearchBar from './ReleaseSearchBar.vue'
import type { ReleaseImportanceFilter, ReleaseStatusFilter, ViewMode } from './releaseTypes'

// 兼容旧的 ReleaseSearchBar 公共接口，同时把 ReleaseTab 的工具栏职责命名为独立组件。
const props = defineProps<{
  modelValue: string
  statusFilter: ReleaseStatusFilter
  importanceFilter: ReleaseImportanceFilter
  viewMode: ViewMode
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
  'update:importanceFilter': [value: ReleaseImportanceFilter]
  'update:viewMode': [value: ViewMode]
  searchEnter: []
}>()

function updateStatusFilter(value: string) {
  emit('update:statusFilter', value as ReleaseStatusFilter)
}

function updateImportanceFilter(value: string) {
  emit('update:importanceFilter', value as ReleaseImportanceFilter)
}

function updateViewMode(value: string) {
  emit('update:viewMode', value as ViewMode)
}
</script>

<template>
  <ReleaseSearchBar
    :model-value="props.modelValue"
    :status-filter="props.statusFilter"
    :importance-filter="props.importanceFilter"
    :view-mode="props.viewMode"
    :show-search="true"
    @update:model-value="emit('update:modelValue', $event)"
    @update:status-filter="updateStatusFilter"
    @update:importance-filter="updateImportanceFilter"
    @update:view-mode="updateViewMode"
    @search-enter="emit('searchEnter')"
  />
</template>
