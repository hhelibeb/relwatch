import { computed, ref } from 'vue'
import { type NotificationStatus, type ReleaseInfo, setNotificationState, setReleaseFlag } from '../api/releases'
import { copyImageToClipboard } from '../api/client'
import type { AgentWorkspaceSeed } from '../injection-keys'
import { track } from './useUsageTracking'
import { t } from '../i18n'
import { releaseFlagColor, releaseFlagged, releaseFlagTitle } from '../utils/releaseFlag'

export interface UseReleaseActionsOptions {
  release: () => ReleaseInfo
  showToast?: (msg: string) => void
  openAgentWorkspace: (seed?: AgentWorkspaceSeed) => void
  onUpdated: () => void
  onBeforeAction?: () => void
  onAfterSendToAgent?: () => void
}

export function useReleaseActions(options: UseReleaseActionsOptions) {
  const isUpdating = ref(false)

  const flagged = computed(() => releaseFlagged(options.release()))
  const flagColor = computed(() => releaseFlagColor(options.release()))
  const flagTitle = computed(() => releaseFlagTitle(options.release().flag, t))

  async function applyFlag(flag: number) {
    options.onBeforeAction?.()
    track('release.flag')
    try {
      await setReleaseFlag(options.release().id, flag)
      options.onUpdated()
    } catch (e: unknown) {
      options.showToast?.(t('release.flag_failed') + (e instanceof Error ? e.message : String(e)))
    }
  }

  async function updateStatus(status: NotificationStatus, minutes?: number) {
    if (isUpdating.value) return
    options.onBeforeAction?.()
    isUpdating.value = true
    track(status === 'snoozed' ? 'release.snooze' : status === 'ignored' ? 'release.ignore' : 'release.mark_read')
    try {
      await setNotificationState(options.release().id, status, minutes)
      if (status === 'snoozed') options.showToast?.(t('release.snooze_scheduled'))
      else if (status === 'ignored') options.showToast?.(t('release.notification_cancelled'))
      else options.showToast?.(t('release.marked_read'))
      options.onUpdated()
    } catch (e: unknown) {
      options.showToast?.(t('release.status_failed') + (e instanceof Error ? e.message : String(e)))
    } finally {
      isUpdating.value = false
    }
  }

  function sendToAgent() {
    options.onBeforeAction?.()
    track('release.send_agent')
    options.openAgentWorkspace({ entities: [{ kind: 'release', id: options.release().id }] })
    options.onAfterSendToAgent?.()
  }

  async function copyImage(url: string | null | undefined) {
    if (!url) return
    track('release.copy')
    try {
      await copyImageToClipboard(url)
      options.showToast?.(t('release.copied'))
    } catch (e: unknown) {
      options.showToast?.(t('release.copy_image_failed') + (e instanceof Error ? e.message : String(e)))
    }
  }

  return {
    isUpdating,
    flagged,
    flagColor,
    flagTitle,
    applyFlag,
    updateStatus,
    sendToAgent,
    copyImage,
  }
}
