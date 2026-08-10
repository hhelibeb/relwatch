import type { ReleaseInfo } from '../api/releases'

export type ViewMode = 'simple' | 'aggregated' | 'calendar'
// 单个版本卡片/详情弹窗内的内容视图模式：摘要 / 译文 / 原文
export type ReleaseContentMode = 'summary' | 'translated' | 'full'
export type ReleaseStatusFilter = 'all' | 'unread' | 'read'
export type ReleaseImportanceFilter = 'all' | '大' | '中' | '小'
// 来源筛选：'all' 或后端 source_type（UI 选项从 sourceTypeDefs 注册表枚举，新增类型时同步扩展）
export type ReleaseSourceFilter = 'all' | 'github' | 'huggingface' | 'youtube' | 'bilibili'

export interface RepoGroup {
  key: string
  releases: ReleaseInfo[]
}
