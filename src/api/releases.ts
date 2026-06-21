import { invokeI18n, openReleaseUrl } from './client'

export type NotificationStatus = 'pending' | 'snoozed' | 'clicked' | 'ignored'

export interface ReleaseInfo {
  id: number
  source_id: number
  source_type: string
  owner: string
  repo: string
  tag_name: string
  release_name: string
  html_url: string
  published_at: string
  prerelease: boolean
  body: string | null
  detected_at: string
  notification_status: NotificationStatus
  snooze_until: string | null
  ai_summary: string | null
  ai_importance: string | null
  body_translated: string | null
}

export interface PollResult {
  new_releases: ReleaseInfo[]
}

export async function getReleases(): Promise<ReleaseInfo[]> {
  return invokeI18n<ReleaseInfo[]>('get_releases')
}

export async function setNotificationState(
  releaseId: number,
  status: NotificationStatus,
  snoozeMinutes?: number
): Promise<void> {
  const args: Record<string, unknown> = { releaseId, status }
  if (snoozeMinutes !== undefined) args.snoozeMinutes = snoozeMinutes
  return invokeI18n('set_notification_state', args)
}

export async function deleteRelease(releaseId: number): Promise<void> {
  return invokeI18n('delete_release', { releaseId })
}

export async function translateRelease(releaseId: number): Promise<void> {
  return invokeI18n('translate_release', { releaseId })
}

export async function triggerPoll(): Promise<PollResult> {
  return invokeI18n<PollResult>('trigger_poll')
}

export async function checkSingleSource(id: number): Promise<PollResult> {
  return invokeI18n<PollResult>('check_single_source', { id })
}

export async function getPollCountdown(): Promise<number> {
  return invokeI18n<number>('get_poll_countdown')
}

export { openReleaseUrl }
