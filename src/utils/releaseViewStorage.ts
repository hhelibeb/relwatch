import type {
  ViewMode,
  ReleaseStatusFilter,
  ReleaseImportanceFilter,
  ReleaseSourceFilter,
  ReleaseFlagFilter,
  ReleaseVersionFilter,
  SourceType,
} from '../components/releaseTypes'

export interface ReleaseViewState {
  viewMode: ViewMode
  statusFilter: ReleaseStatusFilter
  importanceFilter: ReleaseImportanceFilter
  sourceFilter: ReleaseSourceFilter
  flagFilter: ReleaseFlagFilter
  versionFilter: ReleaseVersionFilter
}

const VIEW_KEY = 'relwatch.release.view'
const EXPANDED_KEY = 'relwatch.release.expandedRepos'

// 穷尽映射：新增 SourceType 时 TS 要求补齐，避免白名单漏项把用户的来源筛选静默降级
const SOURCE_TYPE_KEYS: Record<SourceType, true> = {
  github: true,
  huggingface: true,
  youtube: true,
  bilibili: true,
}
const SOURCE_FILTERS: readonly ReleaseSourceFilter[] = ['all', ...(Object.keys(SOURCE_TYPE_KEYS) as SourceType[])]

export const defaultReleaseViewState: ReleaseViewState = {
  viewMode: 'simple',
  statusFilter: 'all',
  importanceFilter: 'all',
  sourceFilter: 'all',
  flagFilter: 'all',
  versionFilter: 'all',
}

function readJson(key: string): unknown {
  try {
    const raw = localStorage.getItem(key)
    return raw ? JSON.parse(raw) : null
  } catch {
    return null
  }
}

export function loadReleaseViewState(): ReleaseViewState {
  const raw = readJson(VIEW_KEY)
  if (!raw || typeof raw !== 'object') return { ...defaultReleaseViewState }
  const v = raw as Record<string, unknown>
  const pick = <T>(value: unknown, allowed: readonly T[], fallback: T): T =>
    allowed.includes(value as T) ? (value as T) : fallback
  return {
    viewMode: pick(v.viewMode, ['simple', 'aggregated', 'calendar'] as const, 'simple'),
    statusFilter: pick(v.statusFilter, ['all', 'unread', 'read'] as const, 'all'),
    importanceFilter: pick(v.importanceFilter, ['all', '大', '中', '小'] as const, 'all'),
    sourceFilter: pick(v.sourceFilter, SOURCE_FILTERS, 'all'),
    flagFilter: pick(v.flagFilter, ['all', 'flagged', 'unflagged', 1, 2, 3, 4, 5, 6] as const, 'all'),
    versionFilter: pick(v.versionFilter, ['all', 'major', 'minor', 'patch', 'prerelease'] as const, 'all'),
  }
}

export function saveReleaseViewState(state: ReleaseViewState): void {
  try {
    localStorage.setItem(VIEW_KEY, JSON.stringify(state))
  } catch { /* localStorage 不可用时静默降级为内存态 */ }
}

export function loadExpandedRepos(): Set<string> {
  const raw = readJson(EXPANDED_KEY)
  if (!Array.isArray(raw)) return new Set()
  return new Set(raw.filter((x): x is string => typeof x === 'string'))
}

export function saveExpandedRepos(keys: Set<string>): void {
  try {
    localStorage.setItem(EXPANDED_KEY, JSON.stringify([...keys].slice(-500)))
  } catch { /* 同上 */ }
}