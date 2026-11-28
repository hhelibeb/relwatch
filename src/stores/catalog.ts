import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { type Source, listSources, sourceRepoKey } from '../api/sources'
import { type ReleaseInfo, getReleaseCatalog, triggerPoll } from '../api/releases'
import { type AgentConfig, getAgentConfig } from '../api/agent'
import { t } from '../i18n'
import { isUnreadStatus } from '../utils'
import { extractErrorMessage } from '../services/feedback'
import { track } from '../composables/useUsageTracking'
import { useUiStore } from './ui'

const REFRESH_DEBOUNCE_MS = 50

export const useCatalogStore = defineStore('catalog', () => {
  const sources = ref<Source[]>([])
  const releases = ref<ReleaseInfo[]>([])
  const agentConfig = ref<AgentConfig | null>(null)
  const sourcesLoaded = ref(false)
  const releasesLoaded = ref(false)
  const sourcesLoading = ref(false)
  const sourcesError = ref('')
  const releasesLoading = ref(false)
  const releasesError = ref('')
  const polling = ref(false)
  const sourceChecking = ref(false)

  // 请求序号竞态防护：后发先至时丢弃过期响应，避免慢请求覆盖新数据
  let sourcesReqId = 0
  let releasesReqId = 0
  let agentConfigReqId = 0

  const agentEnabled = computed(() => agentConfig.value?.enabled ?? false)

  const unreadReleaseCounts = computed<Record<string, number>>(() => {
    const counts: Record<string, number> = {}
    for (const release of releases.value) {
      if (!isUnreadStatus(release.notification_status, release.snooze_until)) continue
      const key = sourceRepoKey(release.source_type, release.owner, release.repo)
      counts[key] = (counts[key] || 0) + 1
    }
    return counts
  })

  const totalReleaseCounts = computed<Record<string, number>>(() => {
    const counts: Record<string, number> = {}
    for (const release of releases.value) {
      const key = sourceRepoKey(release.source_type, release.owner, release.repo)
      counts[key] = (counts[key] || 0) + 1
    }
    return counts
  })

  async function loadSources(): Promise<void> {
    const reqId = ++sourcesReqId
    sourcesLoading.value = true
    try {
      const data = await listSources()
      if (reqId !== sourcesReqId) return
      sources.value = data
      sourcesLoaded.value = true
      sourcesError.value = ''
    } catch (e: unknown) {
      if (reqId !== sourcesReqId) return
      const detail = extractErrorMessage(e)
      sourcesError.value = t('app.load_failed', detail)
      useUiStore().showToast(t('app.load_failed', detail))
    } finally {
      if (reqId === sourcesReqId) {
        sourcesLoading.value = false
      }
    }
  }

  async function loadReleases(): Promise<void> {
    const reqId = ++releasesReqId
    releasesLoading.value = true
    try {
      const data = await getReleaseCatalog()
      if (reqId !== releasesReqId) return
      releases.value = data
      releasesLoaded.value = true
      releasesError.value = ''
    } catch (e: unknown) {
      if (reqId !== releasesReqId) return
      const detail = extractErrorMessage(e)
      releasesError.value = t('app.load_failed', detail)
      useUiStore().showToast(t('app.load_failed', detail))
    } finally {
      if (reqId === releasesReqId) {
        releasesLoading.value = false
      }
    }
  }

  async function loadAgentConfig(): Promise<void> {
    const reqId = ++agentConfigReqId
    try {
      const cfg = await getAgentConfig()
      if (reqId !== agentConfigReqId) return
      agentConfig.value = cfg
    } catch {
      if (reqId !== agentConfigReqId) return
      agentConfig.value = null
    }
  }

  async function loadAllCatalog(): Promise<void> {
    await Promise.allSettled([loadSources(), loadReleases(), loadAgentConfig()])
  }

  let releasesRefreshTimer: ReturnType<typeof setTimeout> | null = null
  function scheduleLoadReleases(): void {
    if (releasesRefreshTimer !== null) return
    releasesRefreshTimer = setTimeout(() => {
      releasesRefreshTimer = null
      void loadReleases()
    }, REFRESH_DEBOUNCE_MS)
  }

  let catalogRefreshTimer: ReturnType<typeof setTimeout> | null = null
  function scheduleRefreshCatalog(): void {
    if (catalogRefreshTimer !== null) return
    catalogRefreshTimer = setTimeout(() => {
      catalogRefreshTimer = null
      void Promise.allSettled([loadSources(), loadReleases()])
    }, REFRESH_DEBOUNCE_MS)
  }

  function cancelScheduled(): void {
    if (releasesRefreshTimer !== null) {
      clearTimeout(releasesRefreshTimer)
      releasesRefreshTimer = null
    }
    if (catalogRefreshTimer !== null) {
      clearTimeout(catalogRefreshTimer)
      catalogRefreshTimer = null
    }
  }

  function patchRelease(id: number, patch: Partial<ReleaseInfo>): void {
    if (!releases.value.some((r) => r.id === id)) return
    releases.value = releases.value.map((r) => (r.id === id ? { ...r, ...patch } : r))
  }

  function patchReleases(ids: number[], patch: Partial<ReleaseInfo>): void {
    if (ids.length === 0) return
    const idSet = new Set(ids)
    releases.value = releases.value.map((r) => (idSet.has(r.id) ? { ...r, ...patch } : r))
  }

  async function handlePoll(): Promise<void> {
    if (polling.value || sourceChecking.value) return
    const ui = useUiStore()
    track('app.check_now')
    const enabled = sources.value.filter((s) => s.enabled)
    if (enabled.length === 0) {
      ui.showToast(t('app.no_sources'))
      return
    }
    polling.value = true
    try {
      const result = await triggerPoll()
      if (result.new_releases.length === 0) {
        ui.showToast(t('app.already_latest'))
      } else {
        ui.showToast(t('app.new_found', String(result.new_releases.length)))
      }
    } catch (e: unknown) {
      ui.showToast(t('app.check_failed', extractErrorMessage(e)))
    } finally {
      polling.value = false
    }
  }

  return {
    sources,
    releases,
    agentConfig,
    sourcesLoaded,
    releasesLoaded,
    sourcesLoading,
    sourcesError,
    releasesLoading,
    releasesError,
    polling,
    sourceChecking,
    agentEnabled,
    unreadReleaseCounts,
    totalReleaseCounts,
    loadSources,
    loadReleases,
    loadAgentConfig,
    loadAllCatalog,
    scheduleLoadReleases,
    scheduleRefreshCatalog,
    cancelScheduled,
    patchRelease,
    patchReleases,
    handlePoll,
  }
})
