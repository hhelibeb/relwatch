<script setup lang="ts">
import { ref, computed } from 'vue'
import { t } from '../i18n'

const props = defineProps<{
  modelValue: string
  statusFilter: string
  importanceFilter: string
  viewMode: string
  showSearch?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'update:statusFilter': [value: string]
  'update:importanceFilter': [value: string]
  'update:viewMode': [value: string]
  searchEnter: []
}>()

// ========== 筛选下拉状态 ==========
const openFilter = ref<'status' | 'importance' | null>(null)
let hoverFilterTimer: ReturnType<typeof setTimeout> | null = null

function hoverFilterEnter() {
  if (hoverFilterTimer) {
    clearTimeout(hoverFilterTimer)
    hoverFilterTimer = null
  }
}

function hoverFilterLeave() {
  hoverFilterTimer = setTimeout(() => {
    openFilter.value = null
  }, 120)
}

const importanceDisplayText = computed(() => {
  if (props.importanceFilter === '大') return '🔴 ' + t('release.importance_high')
  if (props.importanceFilter === '中') return '🟡 ' + t('release.importance_medium')
  if (props.importanceFilter === '小') return '🟢 ' + t('release.importance_low')
  return t('release.filter_all')
})

function onSearchEnter() {
  emit('searchEnter')
}
</script>

<template>
  <div class="log-search-row">
    <div v-if="showSearch !== false" class="input-clear-wrap">
      <input
        :value="modelValue"
        :placeholder="t('release.search')"
        class="search-input"
        @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
        @keydown.enter.prevent="onSearchEnter"
      />
      <button v-if="modelValue" type="button" class="input-clear-btn" :title="t('input.clear')" @click="emit('update:modelValue', '')">✕</button>
    </div>
    <div v-else style="flex:1"></div>
    <div class="filter-group" @mouseleave="hoverFilterLeave()">
      <div class="filter-field" @mouseenter="openFilter = 'status'; hoverFilterEnter()">
        <button class="filter-trigger">
          <span class="filter-label">{{ t('tab.status') }}</span>
          <span class="filter-value" :style="{ color: props.statusFilter === 'unread' ? 'var(--primary)' : props.statusFilter === 'read' ? 'var(--success)' : 'var(--text-muted)' }">{{ props.statusFilter === 'all' ? t('release.filter_all') : (props.statusFilter === 'unread' ? t('release.filter_unread') : t('release.filter_read')) }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'status'" class="filter-dropdown" @mouseenter="hoverFilterEnter()" @mouseleave="hoverFilterLeave()">
          <button :class="{ selected: props.statusFilter === 'all' }" @click="emit('update:statusFilter', 'all'); openFilter = null">{{ t('release.filter_all') }}</button>
          <button :class="{ selected: props.statusFilter === 'unread' }" @click="emit('update:statusFilter', 'unread'); openFilter = null">{{ t('release.filter_unread') }}</button>
          <button :class="{ selected: props.statusFilter === 'read' }" @click="emit('update:statusFilter', 'read'); openFilter = null">{{ t('release.filter_read') }}</button>
        </div>
      </div>
      <div class="filter-divider"></div>
      <div class="filter-field" @mouseenter="openFilter = 'importance'; hoverFilterEnter()">
        <button class="filter-trigger">
          <span class="filter-label">{{ t('tab.importance') }}</span>
          <span class="filter-value" :style="{ color: props.importanceFilter !== 'all' ? 'var(--text)' : 'var(--text-muted)' }">{{ importanceDisplayText }}</span>
          <svg class="filter-arrow" width="12" height="12"><use href="/icons.svg#chevron-down-icon"/></svg>
        </button>
        <div v-if="openFilter === 'importance'" class="filter-dropdown" @mouseenter="hoverFilterEnter()" @mouseleave="hoverFilterLeave()">
          <button :class="{ selected: props.importanceFilter === 'all' }" @click="emit('update:importanceFilter', 'all'); openFilter = null">{{ t('release.filter_all') }}</button>
          <button :class="{ selected: props.importanceFilter === '大' }" @click="emit('update:importanceFilter', '大'); openFilter = null">🔴 {{ t('release.importance_high') }}</button>
          <button :class="{ selected: props.importanceFilter === '中' }" @click="emit('update:importanceFilter', '中'); openFilter = null">🟡 {{ t('release.importance_medium') }}</button>
          <button :class="{ selected: props.importanceFilter === '小' }" @click="emit('update:importanceFilter', '小'); openFilter = null">🟢 {{ t('release.importance_low') }}</button>
        </div>
      </div>
    </div>
    <div class="view-tabs">
      <button :class="{ active: props.viewMode === 'simple' }" @click="emit('update:viewMode', 'simple')">
        <svg><use href="/icons.svg#list-icon"/></svg>
        {{ t('release.view_simple') }}
      </button>
      <button :class="{ active: props.viewMode === 'aggregated' }" @click="emit('update:viewMode', 'aggregated')">
        <svg><use href="/icons.svg#grid-icon"/></svg>
        {{ t('release.view_aggregated') }}
      </button>
      <button :class="{ active: props.viewMode === 'calendar' }" @click="emit('update:viewMode', 'calendar')">
        <svg><use href="/icons.svg#calendar-icon"/></svg>
        {{ t('release.view_calendar') }}
      </button>
    </div>
  </div>
</template>
