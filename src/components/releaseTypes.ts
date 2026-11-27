import type { ReleaseInfo } from '../api/releases'
import type { SourceType } from '../api/source-registry'

export type { SourceType }

export type ViewMode = 'simple' | 'aggregated' | 'calendar'
// 单个版本卡片/详情弹窗内的内容视图模式：摘要 / 译文 / 原文
export type ReleaseContentMode = 'summary' | 'translated' | 'full'
export type ReleaseStatusFilter = 'all' | 'unread' | 'read'
export type ReleaseImportanceFilter = 'all' | '大' | '中' | '小'
// 旗标筛选：'all' / 已标记 / 未标记 / 具体颜色（1-6，与后端 releases.flag 对应）
export type ReleaseFlagFilter = 'all' | 'flagged' | 'unflagged' | 1 | 2 | 3 | 4 | 5 | 6
export type ReleaseVersionFilter = 'all' | 'major' | 'minor' | 'patch' | 'prerelease'
export type ReleaseSourceFilter = 'all' | SourceType

export interface RepoGroup {
  key: string
  releases: ReleaseInfo[]
}
