import type { ReleaseInfo } from '../api/releases'

export type ViewMode = 'simple' | 'aggregated' | 'calendar'
export type ReleaseStatusFilter = 'all' | 'unread' | 'read'
export type ReleaseImportanceFilter = 'all' | '大' | '中' | '小'

export interface RepoGroup {
  key: string
  releases: ReleaseInfo[]
}

export interface CalendarCell {
  date: number
  key: string
  count: number
  isCurrentMonth: boolean
  isToday: boolean
  releases: ReleaseInfo[]
}
