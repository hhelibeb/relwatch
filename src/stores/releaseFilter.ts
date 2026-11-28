import { ref, watch } from 'vue'
import { defineStore } from 'pinia'
import type {
  ViewMode,
  ReleaseStatusFilter,
  ReleaseImportanceFilter,
  ReleaseSourceFilter,
  ReleaseFlagFilter,
  ReleaseVersionFilter,
} from '../components/releaseTypes'
import {
  loadReleaseViewState,
  saveReleaseViewState,
  loadExpandedRepos,
  saveExpandedRepos,
} from '../utils/releaseViewStorage'
import { t } from '../i18n'
import { useUiStore } from './ui'

export const useReleaseFilterStore = defineStore('releaseFilter', () => {
  const initial = loadReleaseViewState()

  const search = ref('')
  const viewMode = ref<ViewMode>(initial.viewMode)
  const statusFilter = ref<ReleaseStatusFilter>(initial.statusFilter)
  const importanceFilter = ref<ReleaseImportanceFilter>(initial.importanceFilter)
  const sourceFilter = ref<ReleaseSourceFilter>(initial.sourceFilter)
  const flagFilter = ref<ReleaseFlagFilter>(initial.flagFilter)
  const versionFilter = ref<ReleaseVersionFilter>(initial.versionFilter)
  const expandedRepos = ref<Set<string>>(loadExpandedRepos())

  // 通知下钻定位状态机
  const focusTarget = ref<number | null>(null)
  const focusToken = ref(0)

  // 挂起持久化计数：允许批量临时重置筛选而不覆盖用户持久化的默认视图偏好
  let persistSuspendDepth = 0

  function runWithoutPersist(fn: () => void): void {
    persistSuspendDepth++
    try {
      fn()
    } finally {
      persistSuspendDepth--
    }
  }

  function hydrateFromStorage(): void {
    const saved = loadReleaseViewState()
    runWithoutPersist(() => {
      viewMode.value = saved.viewMode
      statusFilter.value = saved.statusFilter
      importanceFilter.value = saved.importanceFilter
      sourceFilter.value = saved.sourceFilter
      flagFilter.value = saved.flagFilter
      versionFilter.value = saved.versionFilter
    })
    expandedRepos.value = loadExpandedRepos()
  }

  function persistViewState(): void {
    if (persistSuspendDepth > 0) return
    saveReleaseViewState({
      viewMode: viewMode.value,
      statusFilter: statusFilter.value,
      importanceFilter: importanceFilter.value,
      sourceFilter: sourceFilter.value,
      flagFilter: flagFilter.value,
      versionFilter: versionFilter.value,
    })
  }

  watch(
    [viewMode, statusFilter, importanceFilter, sourceFilter, flagFilter, versionFilter],
    () => persistViewState(),
    { flush: 'sync' },
  )

  function toggleExpandedRepo(key: string): void {
    const next = new Set(expandedRepos.value)
    if (next.has(key)) next.delete(key)
    else next.add(key)
    expandedRepos.value = next
    saveExpandedRepos(next)
  }

  function setExpandedRepo(key: string, expanded: boolean): void {
    const next = new Set(expandedRepos.value)
    if (expanded) next.add(key)
    else next.delete(key)
    expandedRepos.value = next
    saveExpandedRepos(next)
  }

  function openSourceReleases(query: string): void {
    search.value = query
    statusFilter.value = 'all'
    useUiStore().activeTab = 'releases'
  }

  function openSourceUnreadReleases(query: string): void {
    search.value = query
    statusFilter.value = 'unread'
    useUiStore().activeTab = 'releases'
  }

  function focusReleaseById(id: number): void {
    focusTarget.value = id
    focusToken.value++
    useUiStore().activeTab = 'releases'
  }

  function consumeFocus(): void {
    focusTarget.value = null
  }

  function reportFocusNotFound(): void {
    if (focusTarget.value === null) return
    focusTarget.value = null
    useUiStore().showToast(t('release.focus_missing'))
  }

  return {
    search,
    viewMode,
    statusFilter,
    importanceFilter,
    sourceFilter,
    flagFilter,
    versionFilter,
    expandedRepos,
    focusTarget,
    focusToken,
    runWithoutPersist,
    hydrateFromStorage,
    persistViewState,
    toggleExpandedRepo,
    setExpandedRepo,
    openSourceReleases,
    openSourceUnreadReleases,
    focusReleaseById,
    consumeFocus,
    reportFocusNotFound,
  }
})
