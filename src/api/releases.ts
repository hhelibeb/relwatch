import { invokeI18nFn } from './client'
import { commands } from '../bindings'
import type { PollResult, ReleaseInfo } from '../bindings'

// 类型与命令签名由 tauri-specta 从 Rust 生成（src/bindings.ts），此处 re-export 保持调用方路径不变
export type { PollResult, ReleaseInfo } from '../bindings'

export type NotificationStatus = 'pending' | 'snoozed' | 'clicked' | 'ignored'

export async function getReleases(): Promise<ReleaseInfo[]> {
  return invokeI18nFn(commands.getReleases)
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
