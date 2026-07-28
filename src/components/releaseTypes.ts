import type { ReleaseInfo } from '../api/releases'

export type ViewMode = 'simple' | 'aggregated' | 'calendar'
// 单个版本卡片/详情弹窗内的内容视图模式：摘要 / 译文 / 原文
export type ReleaseContentMode = 'summary' | 'translated' | 'full'
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
