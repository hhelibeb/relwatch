import { computed, ref, shallowRef, watch, type Ref } from 'vue'
import { getReleaseDetail, setNotificationState, type ReleaseInfo } from '../api/releases'
import { isUnreadStatus, type AutoReadDetailMode } from '../utils'
import { track } from './useUsageTracking'

export function useReleaseDetailNav(opts: {
  releases: () => ReleaseInfo[]
  autoReadDetail: Ref<AutoReadDetailMode>
  onUpdated: () => void
}) {
  const detailReleaseId = ref<number | null>(null)
  const detailSequenceIds = ref<number[]>([])
  const detailFull = shallowRef<ReleaseInfo | null>(null)
  let detailToken = 0
  const autoMarkingIds = new Set<number>()

  const detailIndex = computed(() => detailSequenceIds.value.indexOf(detailReleaseId.value ?? -1))
  const detailRelease = computed(() => {
    const id = detailReleaseId.value
    if (id === null) return null
    const item = opts.releases().find((r) => r.id === id)
    if (!item) return null
    const full = detailFull.value
    if (!full || full.id !== id) return item
    return {
      ...item,
      body: full.body ?? item.body,
      body_translated: full.body_translated ?? item.body_translated,
    }
  })

  async function loadReleaseDetail(id: number): Promise<void> {
    const token = ++detailToken
    try {
      const full = await getReleaseDetail(id)
      if (token !== detailToken || detailReleaseId.value !== id) return
      detailFull.value = full
    } catch {
      // 取全文失败时保留目录预览
    }
  }

  function autoMarkRead(release: ReleaseInfo): void {
    if (!isUnreadStatus(release.notification_status, release.snooze_until)) return
    if (autoMarkingIds.has(release.id)) return
    autoMarkingIds.add(release.id)
    setNotificationState(release.id, 'clicked')
      .then(() => opts.onUpdated())
      .catch(() => {})
      .finally(() => autoMarkingIds.delete(release.id))
  }

  function openReleaseDetail(release: ReleaseInfo, sequence: ReleaseInfo[]): void {
    detailSequenceIds.value = sequence.map((r) => r.id)
    detailReleaseId.value = release.id
    if (opts.autoReadDetail.value !== 'off') autoMarkRead(release)
  }

  function closeReleaseDetail(): void {
    detailReleaseId.value = null
    detailSequenceIds.value = []
  }

  function navigateReleaseDetail(delta: number): void {
    track(delta < 0 ? 'release.detail_prev' : 'release.detail_next')
    const nextId = detailSequenceIds.value[detailIndex.value + delta]
    if (nextId === undefined) return
    detailReleaseId.value = nextId
    if (opts.autoReadDetail.value === 'navigate') {
      const next = opts.releases().find((r) => r.id === nextId)
      if (next) autoMarkRead(next)
    }
  }

  watch(detailReleaseId, (id) => {
    detailFull.value = null
    if (id !== null) void loadReleaseDetail(id)
  })

  watch(opts.releases, (list) => {
    const id = detailReleaseId.value
    if (id === null) return
    if (!list.some((r) => r.id === id)) {
      closeReleaseDetail()
      return
    }
    void loadReleaseDetail(id)
  })

  return {
    detailReleaseId,
    detailSequenceIds,
    detailIndex,
    detailRelease,
    openReleaseDetail,
    closeReleaseDetail,
    navigateReleaseDetail,
  }
}
