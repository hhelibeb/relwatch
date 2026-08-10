import type { ReleaseInfo } from '../api/releases'

export type ViewMode = 'simple' | 'aggregated' | 'calendar'
// 单个版本卡片/详情弹窗内的内容视图模式：摘要 / 译文 / 原文
export type ReleaseContentMode = 'summary' | 'translated' | 'full'
export type ReleaseStatusFilter = 'all' | 'unread' | 'read'
export type ReleaseImportanceFilter = 'all' | '大' | '中' | '小'
// 后端支持的 source_type 枚举（与 source-registry 注册表、后端 AuthKind 对应）
export type SourceType = 'github' | 'huggingface' | 'youtube' | 'bilibili'
// 来源筛选：'all' 或后端 source_type（UI 选项从 sourceTypeDefs 注册表枚举）
export type ReleaseSourceFilter = 'all' | SourceType

export interface RepoGroup {
  key: string
  releases: ReleaseInfo[]
}
