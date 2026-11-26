import { computed, nextTick, onUnmounted, ref, shallowRef, watch, type Ref } from 'vue'
import {
  getReleaseSearchBodies,
  getReleaseSearchBodiesByIds,
  type ReleaseInfo,
  type ReleaseSearchBody,
} from '../api/releases'
import { mergeBodyIndex, type BodyIndex } from '../utils'

const TIER2_CHAR_BUDGET = 1_500_000
const TIER2_CHUNK_CHARS = 512 * 1024
const BODY_CURSOR_START = Number.MAX_SAFE_INTEGER

function bodySignal(r: ReleaseInfo): string {
  return `${r.body ? 1 : 0}${r.body_translated ? 1 : 0}`
}

function minIdOf(chunk: readonly ReleaseSearchBody[]): number {
  let min = Number.MAX_SAFE_INTEGER
  for (const c of chunk) if (c.id < min) min = c.id
  return min
}

function yieldFrame(): Promise<void> {
  return nextTick().then(
    () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())),
  )
}

export interface UseReleaseDeepSearchOptions {
  releases: () => readonly ReleaseInfo[]
  searchQuery: Ref<string>
}

export function useReleaseDeepSearch(options: UseReleaseDeepSearchOptions) {
  const deepSearch = ref(false)
  const bodyIndex = shallowRef<BodyIndex>(new Map())
  const deepSearching = ref(false)
  const coversAllBodies = ref(false)
  const bodyFillDone = ref(false)
  const bodyTruncated = computed(() => bodyFillDone.value && !coversAllBodies.value)

  let bodyCursor = BODY_CURSOR_START
  let bodyTop = 0
  let bodyIndexChars = 0
  let disposed = false

  const bodyChars = new Map<number, number>()
  const bodySignals = new Map<number, string>()

  function maxCatalogId(): number {
    let max = 0
    for (const r of options.releases()) if (r.id > max) max = r.id
    return max
  }

  function applyChunk(chunk: readonly ReleaseSearchBody[]) {
    if (chunk.length === 0) return
    bodyIndex.value = mergeBodyIndex(bodyIndex.value, chunk)
    for (const c of chunk) {
      const chars = (c.body?.length ?? 0) + (c.body_translated?.length ?? 0)
      bodyIndexChars += chars - (bodyChars.get(c.id) ?? 0)
      bodyChars.set(c.id, chars)
    }
  }

  async function prefetchBodies(maxId: number) {
    bodyTop = Math.max(bodyTop, maxId)
    try {
      while (!disposed) {
        const chunk = await getReleaseSearchBodies(bodyCursor, TIER2_CHUNK_CHARS)
        if (disposed) return
        if (chunk.length === 0) {
          coversAllBodies.value = true
          bodyCursor = 0
          bodyFillDone.value = true
          return
        }
        applyChunk(chunk)
        bodyCursor = minIdOf(chunk)
        if (bodyIndexChars >= TIER2_CHAR_BUDGET) {
          bodyFillDone.value = true
          return
        }
        await yieldFrame()
      }
    } catch {
      // 取正文失败：保留已入索引的部分，不置 bodyFillDone，下次刷新会重试
    }
  }

  function evictOldestBodies() {
    if (bodyIndexChars <= TIER2_CHAR_BUDGET) return
    const ids = [...bodyChars.keys()].sort((a, b) => a - b)
    const next = new Map(bodyIndex.value)
    for (const id of ids) {
      if (bodyIndexChars <= TIER2_CHAR_BUDGET || next.size <= 1) break
      bodyIndexChars -= bodyChars.get(id) ?? 0
      bodyChars.delete(id)
      next.delete(id)
      if (id >= bodyCursor) bodyCursor = id + 1
      coversAllBodies.value = false
    }
    bodyIndex.value = next
  }

  async function refreshChangedBodies(maxId: number) {
    const refetch: number[] = []
    const drop: number[] = []
    const nextSignals = new Map<number, string>()
    for (const r of options.releases()) {
      if (r.id < bodyCursor) continue
      const sig = bodySignal(r)
      nextSignals.set(r.id, sig)
      if (r.id > bodyTop) {
        if (sig !== '00') refetch.push(r.id)
        continue
      }
      const prev = bodySignals.get(r.id)
      if (prev === undefined || prev === sig) continue
      if (sig === '00') drop.push(r.id)
      else refetch.push(r.id)
    }
    bodyTop = Math.max(bodyTop, maxId)
    bodySignals.clear()
    for (const [id, sig] of nextSignals) bodySignals.set(id, sig)

    if (drop.length > 0) {
      const next = new Map(bodyIndex.value)
      for (const id of drop) {
        bodyIndexChars -= bodyChars.get(id) ?? 0
        bodyChars.delete(id)
        next.delete(id)
      }
      bodyIndex.value = next
    }

    for (let i = 0; i < refetch.length; i += 500) {
      const chunk = await getReleaseSearchBodiesByIds(refetch.slice(i, i + 500))
      if (disposed) return
      applyChunk(chunk)
    }

    evictOldestBodies()
  }

  let syncRunning = false
  let syncAgain = false

  async function syncBodyIndex() {
    if (disposed || !deepSearch.value) return
    if (syncRunning) {
      syncAgain = true
      return
    }
    syncRunning = true
    deepSearching.value = true
    try {
      do {
        syncAgain = false
        const maxId = maxCatalogId()
        if (!bodyFillDone.value && bodyIndexChars < TIER2_CHAR_BUDGET) {
          await prefetchBodies(maxId)
        }
        if (disposed || !deepSearch.value) return
        await refreshChangedBodies(maxId)
      } while (syncAgain && !disposed && deepSearch.value)
    } finally {
      syncRunning = false
      deepSearching.value = false
    }
  }

  let bodyIndexSyncRaf = 0
  function scheduleBodyIndexSync() {
    if (bodyIndexSyncRaf !== 0 || disposed) return
    bodyIndexSyncRaf = requestAnimationFrame(() => {
      bodyIndexSyncRaf = 0
      void syncBodyIndex()
    })
  }

  watch(options.releases, () => {
    if (deepSearch.value) scheduleBodyIndexSync()
  })

  watch(options.searchQuery, (q) => {
    if (!q.trim()) deepSearch.value = false
  })

  async function runDeepSearch() {
    if (!options.searchQuery.value.trim()) return
    await syncBodyIndex()
  }

  async function onDeepSearchToggle(on: boolean) {
    deepSearch.value = on
    if (!on) return
    await nextTick()
    void runDeepSearch()
  }

  function enableDeepSearch() {
    deepSearch.value = true
    void runDeepSearch()
  }

  onUnmounted(() => {
    disposed = true
    if (bodyIndexSyncRaf !== 0) {
      cancelAnimationFrame(bodyIndexSyncRaf)
      bodyIndexSyncRaf = 0
    }
    bodyIndex.value = new Map()
    bodyChars.clear()
    bodySignals.clear()
  })

  return {
    deepSearch,
    bodyIndex,
    deepSearching,
    bodyTruncated,
    onDeepSearchToggle,
    enableDeepSearch,
  }
}
