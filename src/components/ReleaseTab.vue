<script setup lang="ts">
import { computed, inject, nextTick, onUnmounted, ref, shallowRef, watch } from 'vue'
import type { ReleaseInfo, ReleaseSearchBody } from '../api/releases'
import { getReleaseDetail, getReleaseSearchBodies, getReleaseSearchBodiesByIds } from '../api/releases'
import { isReadStatus, isUnreadStatus, filterReleaseIndices, mergeBodyIndex, type BodyIndex } from '../utils'
import ReleaseAggregatedList from './ReleaseAggregatedList.vue'
import ReleaseCalendar from './ReleaseCalendar.vue'
import ReleaseDateDetail from './ReleaseDateDetail.vue'
import ReleaseDetailModal from './ReleaseDetailModal.vue'
import ReleaseSearchBar from './ReleaseSearchBar.vue'
import ReleaseSimpleList from './ReleaseSimpleList.vue'
import type { ReleaseFlagFilter, ReleaseImportanceFilter, ReleaseSourceFilter, ReleaseStatusFilter, ReleaseVersionFilter, ViewMode } from './releaseTypes'
import { releaseFlagged } from '../utils/releaseFlag'
import { track } from '../composables/useUsageTracking'
import { ShowImportanceKey } from '../injection-keys'

// 通知定位（App.vue focus-release 事件）下钻到单条 release（评审 P1-1 修复）：
// - focusTarget：目标 release id（App 在收到通知点击时设置，供本组件消费）；
// - focusToken：递增令牌。App 与 release 列表的数据刷新（轮询/标记/删除…）解耦，
//   本组件在“可能使列表就绪/内容变化”的时机统一检查 token 是否变化并消费（只消费最新一次）。
// - focus-consumed：定位成功（目标已展示、滚动、高亮）后置位，交由 App 清空目标。
// - focus-not-found：目标在本列表（重置筛选后）找不到；由 App 判定后给出 Toast（见 App.vue）。
type ReleaseSimpleListHandle = InstanceType<typeof ReleaseSimpleList> & {
  focusReleaseId: (id: number) => boolean
}
type AggregatedListInstance = InstanceType<typeof ReleaseAggregatedList> & {
  expandAll: () => void
}

const props = withDefaults(defineProps<{
  releases: ReleaseInfo[]
  search?: string
  statusFilter?: ReleaseStatusFilter
  /** 通知定位目标 release id（App 在 focus-release 事件时设置）。 */
  focusTarget?: number | null
  /** 定位令牌：每次目标变化时递增；本组件据此决定是否消费。 */
  focusToken?: number
}>(), {
  search: '',
  statusFilter: 'all',
  focusTarget: null,
  focusToken: 0,
})

const emit = defineEmits<{
  update: []
  'update:search': [value: string]
  'update:statusFilter': [value: ReleaseStatusFilter]
  /** 目标已定位并展示。 */
  'focus-consumed': []
  /** 目标在本列表（重置筛选后）仍找不到（已删除）。 */
  'focus-not-found': []
}>()

const viewMode = ref<ViewMode>('simple')
// 「显示重要度」开关（App.vue provide）：关闭时不参与过滤，并清掉残留的重要度筛选
const showImportance = inject(ShowImportanceKey, ref(false))
watch(showImportance, (visible) => {
  if (!visible) importanceFilter.value = 'all'
})
const importanceFilter = ref<ReleaseImportanceFilter>('all')
const sourceFilter = ref<ReleaseSourceFilter>('all')
const flagFilter = ref<ReleaseFlagFilter>('all')
const versionFilter = ref<ReleaseVersionFilter>('all')
const selectedDate = ref<string | null>(null)
const calendarYear = ref(new Date().getFullYear())
const calendarMonth = ref(new Date().getMonth() + 1)
const simpleList = ref<ReleaseSimpleListHandle | null>(null)
const aggregatedList = ref<AggregatedListInstance | null>(null)

// ── 通知定位消费（P1-1）──────────────
// 只消费**最新一次**目标：lastConsumedToken 记录已成功处理的 token，重复触发不会重复滚动/高亮。
let lastConsumedToken = -1

// 等待一帧让重置后的筛选/视图渲染、虚拟列表可视行挂载完成。
function waitForListReady(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

async function doFocusTarget(targetId: number) {
  const token = props.focusToken
  const release = props.releases.find(r => r.id === targetId)
  if (!release) {
    // 目标不在当前数据中：若列表已有数据（加载完成过）说明目标确实不存在（已删除），
    // 上报 App 提示；若列表仍为空（冷启动尚未加载完成）则不消费，等数据 watch 重试。
    if (props.releases.length > 0 && token > lastConsumedToken) {
      lastConsumedToken = token
      emit('focus-not-found')
    }
    return
  }
  // 重置全部筛选并切到简单视图：目标必须不被任何过滤隐藏，且视图支持按 id 滚动定位。
  releaseSearch.value = ''
  statusFilter.value = 'all'
  importanceFilter.value = 'all'
  sourceFilter.value = 'all'
  flagFilter.value = 'all'
  versionFilter.value = 'all'
  viewMode.value = 'simple'
  selectedDate.value = null
  await waitForListReady()
  if (props.focusTarget !== targetId || props.focusToken !== token) {
    // 期间 App 已清空或来了更新的目标：丢弃本次（避免旧目标覆盖新目标的高亮）
    return
  }
  // 简单视图列表可能因虚拟化只渲染可视区：先滚到目标再置高亮（等待行挂载）。
  // 首次可能失败：目标存在于 props.releases（未过滤），但 SimpleList 用的是
  // filteredReleases，视图刚从 aggregated/calendar 切回或列表重建时其 ref 尚未就绪。
  // 因此再等一帧重试一次；仍失败则一律按「定位不到」上报——任何情况下都不留静默失败。
  let handled = simpleList.value?.focusReleaseId?.(targetId) ?? false
  if (!handled) {
    await nextTick()
    if (props.focusTarget !== targetId || props.focusToken !== token) return
    handled = simpleList.value?.focusReleaseId?.(targetId) ?? false
  }
  lastConsumedToken = token
  if (handled) {
    emit('focus-consumed')
  } else {
    emit('focus-not-found')
  }
}

// 定位时机：目标 token 变化 / 数据（releases 引用）变化都可能使“目标可定位”。
// 消费幂等（只处理最新 token，成功才记录），重复触发不会重复滚动/高亮。
function consumeFocusIfPending() {
  const target = props.focusTarget
  if (target === null || props.focusToken <= lastConsumedToken) return
  void doFocusTarget(target)
}

watch(() => props.focusToken, consumeFocusIfPending, { flush: 'post' })
watch(() => props.releases, consumeFocusIfPending, { flush: 'post' })

const releaseSearch = computed({
  get: () => props.search,
  set: (value: string) => emit('update:search', value),
})

const statusFilter = computed({
  get: () => props.statusFilter,
  set: (value: ReleaseStatusFilter) => emit('update:statusFilter', value),
})

const hasActiveFilter = computed(() => {
  return releaseSearch.value.trim() !== '' || statusFilter.value !== 'all' || (showImportance.value && importanceFilter.value !== 'all') || sourceFilter.value !== 'all' || flagFilter.value !== 'all' || versionFilter.value !== 'all'
})

const filteredReleases = computed(() => {
  let list = props.releases

  const q = releaseSearch.value.trim()
  if (q) {
    // 深度搜索关闭时不用索引：正文索引在会话内驻留（重复开关不重拉），
    // 是否参与命中必须由 deepSearch 显式决定，不能靠 bodyIndex 是否为空。
    const picked = filterReleaseIndices(list, q, deepSearch.value ? bodyIndex.value : null)
    list = picked.map(i => list[i])
  }

  if (statusFilter.value === 'unread') {
    list = list.filter(release => isUnreadStatus(release.notification_status, release.snooze_until))
  } else if (statusFilter.value === 'read') {
    list = list.filter(release => isReadStatus(release.notification_status))
  }

  if (showImportance.value && importanceFilter.value !== 'all') {
    list = list.filter(release => release.ai_importance === importanceFilter.value)
  }

  if (sourceFilter.value !== 'all') {
    list = list.filter(release => release.source_type === sourceFilter.value)
  }

  if (flagFilter.value === 'flagged') {
    list = list.filter(releaseFlagged)
  } else if (flagFilter.value === 'unflagged') {
    list = list.filter(release => !releaseFlagged(release))
  } else if (flagFilter.value !== 'all') {
    list = list.filter(release => release.flag === flagFilter.value)
  }

  if (versionFilter.value !== 'all') {
    if (versionFilter.value === 'prerelease') {
      list = list.filter(release => release.prerelease)
    } else {
      list = list.filter(release => release.version_bump === versionFilter.value)
    }
  }

  return list
})

// ── 深度搜索（Tier2：GitHub / HF 正文与译文全文）──────────────
// 目录只带正文预览（600 字，见后端 get_release_catalog 契约），全文按 id 游标
// **从新到旧**分块预取，搜索全部留在前端。四条不变量：
//  ① **水位** TIER2_CHAR_BUDGET 是索引正文总量的上界：从最新一块往下填，填到水位即停。
//     方向是刻意的 —— 装不下的必须是更早的内容，否则「搜不到」的恰是用户最常搜的新版本；
//  ② 填到库底 → coversAllBodies，此时水位覆盖全库正文；没填到 → 越界，
//     UI 据 bodyTruncated 显式标注「仅搜索近期正文」，不静默降级；
//  ③ 索引按 release id 建表并**增量维护**：目录刷新只补「新增行」（按 id 取，倒序游标
//     在游标起点之上不回补）与「内容变化行」，不再整体重建 —— 整体重建会让每次轮询/
//     标记已读后的命中结果闪一次；新版本挤入后从最旧一端淘汰，水位仍是上界；
//  ④ 已取到的块在会话内驻留（重复开关深度搜索不重拉），组件卸载即释放。
const TIER2_CHAR_BUDGET = 1_500_000    // 水位：索引正文总字符上界
const TIER2_CHUNK_CHARS = 512 * 1024   // 单次请求正文预算（后端另有 64K–4M 的 clamp）
// 首次游标：大于任何真实 release id（id 是 SQLite 自增 i64，远小于 2^53-1）
const BODY_CURSOR_START = Number.MAX_SAFE_INTEGER

const deepSearch = ref(false)              // 是否处于深度搜索态
const bodyIndex = shallowRef<BodyIndex>(new Map())
const deepSearching = ref(false)           // 取正文 / 维护索引期间的 loading 态
const coversAllBodies = ref(false)         // 水位是否已覆盖全库正文
const bodyFillDone = ref(false)            // 首次预取是否已有定论（预取中不闪越界提示）
/** 能力边界：水位没覆盖全库正文（且预取已告一段落）。 */
const bodyTruncated = computed(() => bodyFillDone.value && !coversAllBodies.value)

// 覆盖窗口 [bodyCursor, bodyTop]（闭区间，id 单调不减）：
//   bodyCursor = 已取到的最小 id，既是窗口下界，也是下一块的排他上界（后端取 id < cursor）
//   bodyTop    = 已见的最大 id，超出它的都是新增行
let bodyCursor = BODY_CURSOR_START
let bodyTop = 0
let bodyIndexChars = 0                     // 已入索引的正文总字符数（水位累加）
let disposed = false                       // 组件已卸载：中止全部在途流程

// id → 入索引的字符数：淘汰/重取正文前要先扣掉旧值，否则水位被重复累加
const bodyChars = new Map<number, number>()
// id → 正文指纹（是否已有 body / 译文）：判定「已存在的行内容变了」的唯一信号
const bodySignals = new Map<number, string>()

/** 正文指纹。「从无到有 / 从有到无」才算内容变化，与目录里的预览长短无关。 */
function bodySignal(r: ReleaseInfo): string {
  return `${r.body ? 1 : 0}${r.body_translated ? 1 : 0}`
}

function maxCatalogId(): number {
  let max = 0
  for (const r of props.releases) if (r.id > max) max = r.id
  return max
}

/** 本块的最小 id：倒序游标的下一站（本块各行都已取到，故可直接作排他上界）。 */
function minIdOf(chunk: readonly ReleaseSearchBody[]): number {
  let min = Number.MAX_SAFE_INTEGER
  for (const c of chunk) if (c.id < min) min = c.id
  return min
}

/** 让出一帧：nextTick 让 Vue 完成本轮渲染，rAF 让浏览器真正绘制。
 *  只有 rAF 能保证连续大响应之间出现绘制机会（微任务链会在同一帧内排空）。 */
function yieldFrame(): Promise<void> {
  return nextTick().then(
    () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())),
  )
}

/** 把一块正文并入索引，并累计字符水位。 */
function applyChunk(chunk: readonly ReleaseSearchBody[]) {
  if (chunk.length === 0) return
  bodyIndex.value = mergeBodyIndex(bodyIndex.value, chunk)
  for (const c of chunk) {
    const chars = (c.body?.length ?? 0) + (c.body_translated?.length ?? 0)
    bodyIndexChars += chars - (bodyChars.get(c.id) ?? 0)
    bodyChars.set(c.id, chars)
  }
}

/** 从最新一块往下填正文，直到填到库底、触及水位或组件卸载。 */
async function prefetchBodies(maxId: number) {
  bodyTop = Math.max(bodyTop, maxId)
  try {
    while (!disposed) {
      const chunk = await getReleaseSearchBodies(bodyCursor, TIER2_CHUNK_CHARS)
      if (disposed) return
      if (chunk.length === 0) {
        // 已到库底：窗口覆盖全库正文，此后只需增量维护
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

/** 目录刷新后的增量维护。
 *
 *  - **新增行**（新 release，id 更大）：倒序游标在起点之上不回补，按 id 取；
 *  - **已存在行内容变化**：翻译落库填 body_translated、HF README 回填 body —— 按 id 重取。
 *
 *  只处理覆盖窗口内的行（`bodyCursor ≤ id`）：水位之外（更旧）的行不参与，否则
 *  每次翻译都会把水位外的正文拉进内存，水位就不再是上界。 */
async function refreshChangedBodies(maxId: number) {
  const refetch: number[] = []
  const drop: number[] = []
  const nextSignals = new Map<number, string>()
  for (const r of props.releases) {
    if (r.id < bodyCursor) continue
    const sig = bodySignal(r)
    nextSignals.set(r.id, sig)
    if (r.id > bodyTop) {
      // 新增行：从没取过，正文非空即取（取回后若超水位由 evictOldestBodies 淘汰最旧）
      if (sig !== '00') refetch.push(r.id)
      continue
    }
    const prev = bodySignals.get(r.id)
    // prev 缺失 = 本次才被游标覆盖，索引内容与目录一致，只登记不重取
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

  // 后端单次至多 500 个 id
  for (let i = 0; i < refetch.length; i += 500) {
    const chunk = await getReleaseSearchBodiesByIds(refetch.slice(i, i + 500))
    if (disposed) return
    applyChunk(chunk)
  }

  evictOldestBodies()
}

/** 新版本挤入后可能超过水位：从最旧一端淘汰，使水位仍是内存上界。
 *  至少保留一条（单条超大正文不能把索引清空），淘汰即水位下界上移。 */
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

/** 索引同步入口：预取新块 + 重取新增/内容变化的行。
 *
 *  重入保护：首次预取可能还在途中（目录已刷新），两个游标循环并发会在同一游标上
 *  重复取块。此时只记账，由当前这一轮收尾时补做。 */
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
      // 只在首次填充未定论时走游标：一旦填到库底或触及水位就不再回溯
      // （否则新版本挤入触发淘汰后又会往下拉更早的正文，来回churn）。
      // 不动 bodyTop：它必须留在「上次见到的最大 id」，新增行才认得出来（见 refreshChangedBodies）。
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

// 同一渲染帧内多次目录刷新（轮询完成 + 标记已读等双路径）合并为一次同步。
let bodyIndexSyncRaf = 0
function scheduleBodyIndexSync() {
  if (bodyIndexSyncRaf !== 0 || disposed) return
  bodyIndexSyncRaf = requestAnimationFrame(() => {
    bodyIndexSyncRaf = 0
    void syncBodyIndex()
  })
}

// 目录引用变化（轮询 / 标记 / 删除后重拉）不再清空索引，只做增量维护。
watch(() => props.releases, () => {
  if (deepSearch.value) scheduleBodyIndexSync()
})
// 搜索词被清空时自动退出深度搜索态（索引保留，下次开启不重拉）
watch(releaseSearch, (q) => {
  if (!q.trim()) deepSearch.value = false
})

async function runDeepSearch() {
  if (!releaseSearch.value.trim()) return
  // 深度搜索态由 onDeepSearchToggle / enableDeepSearch 先行置位；此处只触发同步
  await syncBodyIndex()
}

function onDeepSearchToggle(on: boolean) {
  deepSearch.value = on
  if (on) void runDeepSearch()
}

function enableDeepSearch() {
  deepSearch.value = true
  void runDeepSearch()
}

// 组件卸载（切 tab / 路由离开）时中止在途流程并释放索引：回调与在途响应都可能
// 持有水位内的正文（最多 TIER2_CHAR_BUDGET 字符），不释放会拖慢回收。
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

function handleSearchEnter() {
  if (viewMode.value === 'aggregated') aggregatedList.value?.expandAll()
}

function backToCalendar() {
  selectedDate.value = null
}

function prevMonth() {
  // 下限保护：不允许早于 2010-01，与 nextMonth 的上限保护对称，避免远古日期渲染异常
  const MIN_YEAR = 2010
  if (calendarYear.value <= MIN_YEAR && calendarMonth.value === 1) return
  if (calendarMonth.value === 1) {
    calendarMonth.value = 12
    calendarYear.value--
  } else {
    calendarMonth.value--
  }
}

function nextMonth() {
  const current = new Date()
  const currentYear = current.getFullYear()
  const currentMonth = current.getMonth() + 1
  const nextYear = calendarMonth.value === 12 ? calendarYear.value + 1 : calendarYear.value
  const nextMonthValue = calendarMonth.value === 12 ? 1 : calendarMonth.value + 1
  if (nextYear > currentYear || (nextYear === currentYear && nextMonthValue > currentMonth)) return

  calendarYear.value = nextYear
  calendarMonth.value = nextMonthValue
}

watch(viewMode, () => {
  selectedDate.value = null
})

// ========== 版本详情弹窗 ==========
// 弹窗导航序列由打开时的视图上下文决定（简单=全局时间序，聚合=同仓库，日历=当日），
// 各列表组件通过 open-detail 事件携带。只存 id 序列，release 对象实时从 props.releases
// 查找，保证列表刷新（如翻译完成）后弹窗内容同步更新。
const detailReleaseId = ref<number | null>(null)
const detailSequenceIds = ref<number[]>([])
// 目录里的正文是预览投影，打开详情时按 id 取全文。取到前用列表项（预览）渲染，
// 因此 ReleaseDetailModal 无需感知「预览 / 全文」的区别（替换窗口是一次本地 IPC）。
const detailFull = shallowRef<ReleaseInfo | null>(null)
let detailToken = 0

const detailIndex = computed(() => detailSequenceIds.value.indexOf(detailReleaseId.value ?? -1))
const detailRelease = computed(() => {
  const id = detailReleaseId.value
  if (id === null) return null
  const item = props.releases.find(r => r.id === id)
  if (!item) return null
  const full = detailFull.value
  if (!full || full.id !== id) return item
  return {
    ...item,
    body: full.body ?? item.body,
    body_translated: full.body_translated ?? item.body_translated,
  }
})

async function loadReleaseDetail(id: number) {
  const token = ++detailToken
  try {
    const full = await getReleaseDetail(id)
    // 竞态防护：期间已切换目标 / 关闭弹窗则丢弃
    if (token !== detailToken || detailReleaseId.value !== id) return
    detailFull.value = full
  } catch {
    // 取全文失败不打断阅读：保留目录里的预览
  }
}

function openReleaseDetail(release: ReleaseInfo, sequence: ReleaseInfo[]) {
  detailSequenceIds.value = sequence.map(r => r.id)
  detailReleaseId.value = release.id
}

function closeReleaseDetail() {
  detailReleaseId.value = null
  detailSequenceIds.value = []
}

// 目标变化（打开 / 逐条导航 / 关闭）即取全文：先清掉上一条的全文，避免串内容
watch(detailReleaseId, (id) => {
  detailFull.value = null
  if (id !== null) void loadReleaseDetail(id)
})

// 目录刷新（轮询 / 翻译落库 / 标记）后在打开中的条目上重取全文：翻译完成后同步内容
watch(() => props.releases, () => {
  if (detailReleaseId.value !== null) void loadReleaseDetail(detailReleaseId.value)
})

function navigateReleaseDetail(delta: number) {
  track(delta < 0 ? 'release.detail_prev' : 'release.detail_next')
  const nextId = detailSequenceIds.value[detailIndex.value + delta]
  if (nextId !== undefined) detailReleaseId.value = nextId
}

</script>

<template>
  <section class="tab-content">
    <ReleaseSearchBar
      v-model="releaseSearch"
      v-model:status-filter="statusFilter"
      v-model:importance-filter="importanceFilter"
      v-model:source-filter="sourceFilter"
      v-model:view-mode="viewMode"
      v-model:flag-filter="flagFilter"
      v-model:version-filter="versionFilter"
      :releases="releases"
      :count="filteredReleases.length"
      :deep-search="deepSearch"
      :deep-searching="deepSearching"
      :body-truncated="bodyTruncated"
      @update:deep-search="onDeepSearchToggle"
      @search-enter="handleSearchEnter"
    />

    <ReleaseSimpleList
      v-if="viewMode === 'simple'"
      ref="simpleList"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      :has-search-query="releaseSearch.trim() !== ''"
      :deep-search="deepSearch"
      @enable-deep="enableDeepSearch"
      @update="emit('update')"
      @open-detail="openReleaseDetail"
    />

    <ReleaseAggregatedList
      v-else-if="viewMode === 'aggregated'"
      ref="aggregatedList"
      :releases="filteredReleases"
      :is-filtering="hasActiveFilter"
      @update="emit('update')"
      @open-detail="openReleaseDetail"
    />

    <template v-else-if="viewMode === 'calendar'">
      <ReleaseDateDetail
        v-if="selectedDate !== null"
        :selected-date="selectedDate"
        :releases="filteredReleases"
        @back="backToCalendar"
        @update="emit('update')"
        @open-detail="openReleaseDetail"
      />
      <ReleaseCalendar
        v-else
        :releases="filteredReleases"
        :year="calendarYear"
        :month="calendarMonth"
        @prev-month="prevMonth"
        @next-month="nextMonth"
        @select-date="selectedDate = $event"
      />
    </template>

    <ReleaseDetailModal
      v-if="detailRelease"
      :release="detailRelease"
      :position="detailIndex + 1"
      :total="detailSequenceIds.length"
      :has-prev="detailIndex > 0"
      :has-next="detailIndex >= 0 && detailIndex < detailSequenceIds.length - 1"
      @close="closeReleaseDetail"
      @navigate="navigateReleaseDetail"
      @update="emit('update')"
    />

  </section>
</template>
