import type { ReleaseInfo } from '../api/releases'

export type ViewMode = 'simple' | 'aggregated' | 'calendar'
// 单个版本卡片/详情弹窗内的内容视图模式：摘要 / 译文 / 原文
export type ReleaseContentMode = 'summary' | 'translated' | 'full'
export type ReleaseStatusFilter = 'all' | 'unread' | 'read'
export type ReleaseImportanceFilter = 'all' | '大' | '中' | '小'
// 旗标筛选：'all' / 已标记 / 未标记 / 具体颜色（1-6，与后端 releases.flag 对应）
export type ReleaseFlagFilter = 'all' | 'flagged' | 'unflagged' | 1 | 2 | 3 | 4 | 5 | 6
// 版本类型筛选：基于 semver 的变化类型；'prerelease' 为预发布（复用 prerelease 字段）
export type ReleaseVersionFilter = 'all' | 'major' | 'minor' | 'patch' | 'prerelease'
// 后端支持的 source_type 枚举（与 source-registry 注册表、后端 AuthKind 对应）
export type SourceType = 'github' | 'huggingface' | 'youtube' | 'bilibili'
// 来源筛选：'all' 或后端 source_type（UI 选项从 sourceTypeDefs 注册表枚举）
export type ReleaseSourceFilter = 'all' | SourceType

export interface RepoGroup {
  key: string
  releases: ReleaseInfo[]
}
