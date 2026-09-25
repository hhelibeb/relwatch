import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { PollResult, ReleaseInfo, ReleaseSearchBody } from '../bindings'

// 类型与命令签名由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { PollResult, ReleaseInfo, ReleaseSearchBody } from '../bindings'

export type NotificationStatus = 'pending' | 'snoozed' | 'clicked' | 'ignored'

/** 版本列表数据源：全库目录（正文为预览投影，见后端 get_release_catalog 契约）。 */
export async function getReleaseCatalog(): Promise<ReleaseInfo[]> {
  return invokeI18nFn(commands.getReleaseCatalog)
}

/** 单条 release 全文（详情弹窗用；目录里只有正文预览）。 */
export async function getReleaseDetail(releaseId: number): Promise<ReleaseInfo> {
  return invokeI18nFn(() => commands.getReleaseDetail(releaseId))
}

/**
 * 全文搜索索引的正文分块：按 id 游标取一块（**从新到旧**），`maxChars` 为单次字符预算。
 * 首次传 `BODY_CURSOR_START` 开始；游标取返回值的**最小** id。返回空数组表示已到库底。
 */
export async function getReleaseSearchBodies(
  beforeId: number,
  maxChars: number
): Promise<ReleaseSearchBody[]> {
  return invokeI18nFn(() => commands.getReleaseSearchBodies(beforeId, maxChars))
}

/**
 * 按 id 批量取正文：刷新索引里内容已变化的条目（翻译落库 / README 回填）。
 * 游标分块只覆盖新增行，覆盖不了"已存在的行内容变了"。单次至多 500 个 id。
 */
export async function getReleaseSearchBodiesByIds(
  ids: number[]
): Promise<ReleaseSearchBody[]> {
  if (ids.length === 0) return []
  return invokeI18nFn(() => commands.getReleaseSearchBodiesByIds(ids))
}

export async function setNotificationState(
  releaseId: number,
  status: NotificationStatus,
  snoozeMinutes?: number
): Promise<void> {
  await invokeI18nFn(() => commands.setNotificationState(releaseId, status, snoozeMinutes ?? null))
}

export async function deleteRelease(releaseId: number): Promise<void> {
  await invokeI18nFn(() => commands.deleteRelease(releaseId))
}

/** 设置旗标：0 = 清除，1-6 = 预设颜色。 */
export async function setReleaseFlag(releaseId: number, flag: number): Promise<void> {
  await invokeI18nFn(() => commands.setReleaseFlag(releaseId, flag))
}

export async function translateRelease(releaseId: number): Promise<void> {
  await invokeI18nFn(() => commands.translateRelease(releaseId))
}

export async function triggerPoll(): Promise<PollResult> {
  return invokeI18nFn(commands.triggerPoll)
}

export async function checkSingleSource(id: number): Promise<PollResult> {
  return invokeI18nFn(() => commands.checkSingleSource(id))
}

export async function getPollCountdown(): Promise<number> {
  return invokeI18nFn(commands.getPollCountdown)
}
