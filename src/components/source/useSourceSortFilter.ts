import { computed, ref, watch, type Ref } from 'vue'
import { type Source, sourceRepoKey, sourceTypeDefs } from '../../api/sources'
import { t } from '../../i18n'

const SOURCE_SORT_FIELD_STORAGE_KEY = 'relwatch.source.sort.field'
const SOURCE_SORT_DIRECTION_STORAGE_KEY = 'relwatch.source.sort.direction'

export type SourceSortField = 'default' | 'name' | 'status' | 'type' | 'created'
export type SortDirection = 'asc' | 'desc'

function isSourceSortField(value: string | null): value is SourceSortField {
  return value === 'default' || value === 'name' || value === 'status' || value === 'type' || value === 'created'
}

function isSortDirection(value: string | null): value is SortDirection {
  return value === 'asc' || value === 'desc'
}

function readStoredSourceSortField(): SourceSortField {
  try {
    const value = window.localStorage.getItem(SOURCE_SORT_FIELD_STORAGE_KEY)
    return isSourceSortField(value) ? value : 'default'
  } catch {
    return 'default'
  }
}

function readStoredSourceSortDirection(): SortDirection {
  try {
    const value = window.localStorage.getItem(SOURCE_SORT_DIRECTION_STORAGE_KEY)
    return isSortDirection(value) ? value : 'desc'
  } catch {
    return 'desc'
  }
}

export function sourceMatchesSearch(source: Source, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  const name = `${source.owner}/${source.repo}`.toLowerCase()
  return name.includes(q)
    || source.owner.toLowerCase().includes(q)
    || source.repo.toLowerCase().includes(q)
    || (source.description ?? '').toLowerCase().includes(q)
}

export function useSourceSortFilter(
  sources: Ref<Source[]>,
  unreadReleaseCounts: Ref<Record<string, number>>,
) {
  const sourceSearch = ref('')
  const trimmedSourceSearch = computed(() => sourceSearch.value.trim())
  const hasActiveSourceSearch = computed(() => trimmedSourceSearch.value.length > 0)

  const sourceSortField = ref<SourceSortField>(readStoredSourceSortField())
  const sourceSortDirection = ref<SortDirection>(readStoredSourceSortDirection())

  watch(sourceSortField, (value) => {
    try {
      window.localStorage.setItem(SOURCE_SORT_FIELD_STORAGE_KEY, value)
    } catch { /* ignore unavailable storage */ }
  })

  watch(sourceSortDirection, (value) => {
    try {
      window.localStorage.setItem(SOURCE_SORT_DIRECTION_STORAGE_KEY, value)
    } catch { /* ignore unavailable storage */ }
  })

  const filteredSources = computed(() => {
    const q = trimmedSourceSearch.value
    if (!q) return sources.value
    return sources.value.filter((s) => sourceMatchesSearch(s, q))
  })

  const sortedSources = computed(() => {
    const direction = sourceSortDirection.value === 'asc' ? 1 : -1
    const typeIndex = (type: string) => {
      const idx = sourceTypeDefs.findIndex((d) => d.type === type)
      return idx === -1 ? sourceTypeDefs.length : idx
    }
    return [...filteredSources.value].sort((a, b) => {
      let result: number
      if (sourceSortField.value === 'default') {
        const aPending = unreadReleaseCounts.value[sourceRepoKey(a.source_type, a.owner, a.repo)] || 0
        const bPending = unreadReleaseCounts.value[sourceRepoKey(b.source_type, b.owner, b.repo)] || 0
        result = aPending - bPending || a.id - b.id
      } else if (sourceSortField.value === 'name') {
        result = `${a.owner}/${a.repo}`.localeCompare(`${b.owner}/${b.repo}`)
      } else if (sourceSortField.value === 'status') {
        if (a.enabled !== b.enabled) result = a.enabled ? -1 : 1
        else result = b.id - a.id
      } else if (sourceSortField.value === 'type') {
        result = typeIndex(a.source_type) - typeIndex(b.source_type)
          || `${a.owner}/${a.repo}`.localeCompare(`${b.owner}/${b.repo}`)
          || a.id - b.id
      } else {
        result = a.created_at.localeCompare(b.created_at) || a.id - b.id
      }
      return result * direction
    })
  })

  const sortFieldOptions = computed(() => [
    { value: 'default' as const, label: t('source.sort_default') },
    { value: 'name' as const, label: t('source.sort_name') },
    { value: 'status' as const, label: t('source.sort_status') },
    { value: 'type' as const, label: t('source.sort_type') },
    { value: 'created' as const, label: t('source.sort_created') },
  ])

  const sortDirectionOptions = computed(() => [
    { value: 'asc' as const, label: t('source.sort_asc') },
    { value: 'desc' as const, label: t('source.sort_desc') },
  ])

  const sortLabelText = computed(() => {
    const labels: Record<SourceSortField, string> = {
      default: t('source.sort_default'),
      name: t('source.sort_name'),
      status: t('source.sort_status'),
      type: t('source.sort_type'),
      created: t('source.sort_created'),
    }
    return labels[sourceSortField.value]
  })

  return {
    sourceSearch,
    trimmedSourceSearch,
    hasActiveSourceSearch,
    sourceSortField,
    sourceSortDirection,
    filteredSources,
    sortedSources,
    sortFieldOptions,
    sortDirectionOptions,
    sortLabelText,
  }
}
