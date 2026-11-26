import { onUnmounted, ref, type Ref } from 'vue'
import { type Source, listSources } from '../api/sources'
import { type ReleaseInfo, getReleaseCatalog } from '../api/releases'
import { type AgentConfig, getAgentConfig } from '../api/agent'
import { t } from '../i18n'

export interface AppCatalogStore {
  sources: Ref<Source[]>
  releases: Ref<ReleaseInfo[]>
  agentConfig: Ref<AgentConfig | null>
  sourcesLoaded: Ref<boolean>
  releasesLoaded: Ref<boolean>
  sourcesLoading: Ref<boolean>
  sourcesError: Ref<string>
  releasesLoading: Ref<boolean>
  releasesError: Ref<string>
  loadSources: () => Promise<void>
  loadReleases: () => Promise<void>
  loadAgentConfig: () => Promise<void>
  scheduleLoadReleases: () => void
  scheduleRefreshCatalog: () => void
}

export function useAppCatalog(onError: (msg: string) => void): AppCatalogStore {
  const sources = ref<Source[]>([])
  const releases = ref<ReleaseInfo[]>([])
  const agentConfig = ref<AgentConfig | null>(null)
  const sourcesLoaded = ref(false)
  const releasesLoaded = ref(false)
  const sourcesLoading = ref(false)
  const sourcesError = ref('')
  const releasesLoading = ref(false)
  const releasesError = ref('')

  async function loadSources() {
    sourcesLoading.value = true
    try {
      sources.value = await listSources()
      sourcesLoaded.value = true
      sourcesError.value = ''
    } catch (e: unknown) {
      const detail = e instanceof Error ? e.message : String(e)
      sourcesError.value = t('app.load_failed', detail)
      onError(t('app.load_failed', detail))
    } finally {
      sourcesLoading.value = false
    }
  }

  async function loadReleases() {
    releasesLoading.value = true
    try {
      releases.value = await getReleaseCatalog()
      releasesLoaded.value = true
      releasesError.value = ''
    } catch (e: unknown) {
      const detail = e instanceof Error ? e.message : String(e)
      releasesError.value = t('app.load_failed', detail)
      onError(t('app.load_failed', detail))
    } finally {
      releasesLoading.value = false
    }
  }

  async function loadAgentConfig() {
    try {
      agentConfig.value = await getAgentConfig()
    } catch {
      agentConfig.value = null
    }
  }

  let releasesRefreshTimer: ReturnType<typeof setTimeout> | null = null
  function scheduleLoadReleases() {
    if (releasesRefreshTimer !== null) return
    releasesRefreshTimer = setTimeout(() => {
      releasesRefreshTimer = null
      void loadReleases()
    }, 50)
  }

  let catalogRefreshTimer: ReturnType<typeof setTimeout> | null = null
  function scheduleRefreshCatalog() {
    if (catalogRefreshTimer !== null) return
    catalogRefreshTimer = setTimeout(() => {
      catalogRefreshTimer = null
      void Promise.allSettled([loadSources(), loadReleases()])
    }, 50)
  }

  onUnmounted(() => {
    if (releasesRefreshTimer !== null) {
      clearTimeout(releasesRefreshTimer)
      releasesRefreshTimer = null
    }
    if (catalogRefreshTimer !== null) {
      clearTimeout(catalogRefreshTimer)
      catalogRefreshTimer = null
    }
  })

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
    loadSources,
    loadReleases,
    loadAgentConfig,
    scheduleLoadReleases,
    scheduleRefreshCatalog,
  }
}
