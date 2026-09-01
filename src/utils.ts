import { t, getLocale } from './i18n'
import { getSourceTypeDef } from './api/source-registry'

// 字段与 src/bindings.ts 的 ReleaseInfo 对齐（源描述与 AI 摘要可为 null）
interface SearchableRelease {
  owner: string
  repo: string
  tag_name: string
  release_name: string
  body: string | null
  source_type: string          // ← 决定 body 归属 Tier1 还是 Tier2
  source_description?: string | null
  ai_summary?: string | null
  body_translated?: string | null
}

export function formatDate(dateStr: string): string {
  if (!dateStr) return ''
  const d = new Date(dateStr)
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString(getLocale())
}

export function formatCountdown(secs: number): string {
  if (secs <= 0) return t('app.check_soon')
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return t('app.min_sec', String(m), String(s))
}

/** token 化：trim → 小写 → 按空白切词。
 *  版本号保持整体（v1.14.0 不按点拆分），中文不切词。 */
export function tokenizeQuery(query: string): string[] {
  return query.trim().toLowerCase().split(/\s+/).filter(Boolean)
}

/** body 属于「简介/标签」而非「长正文」的源类型判定（视频源无 AI 摘要，body 是唯一内容载体）。
 *  判据是字段语义，不是长度 —— 见 docs/release-fulltext-search-impl.md §1.4（长度阈值会在同类源内部制造不可预测性）。
 *
 *  能力位**派生自源类型注册表**（aiSummary === false），不在此处镜像类型集合：
 *  注册表是唯一事实来源，`syncSourceCapabilities()` 依后端 `list_source_types` 的 ai_eligible 覆写
 *  aiSummary 后，本判定自动跟随。与 `utils/releaseDisplay.ts::canTranslateRelease` 同源同判据。
 *
 *  已知边界：本判定当前把「不生成 AI 摘要」等价于「body 是短简介」（当下成立的巧合）。
 *  若将来出现「不生成摘要但正文很长」的文本类源，应拆出独立的「body 是否短简介」判据，
 *  而不是回退到硬编码类型集合。 */
function isSummaryBodySource(r: SearchableRelease): boolean {
  return getSourceTypeDef(r.source_type ?? '')?.aiSummary === false
}

/** Tier1 字段（元数据 + AI 摘要 + 视频源简介）。
 *  保留字段边界而非拼成单串——
 *  ① 使 `owner/repo` 可作为独立片段被匹配（关键，见测试中的回归用例）
 *  ② 为未来的字段加权排序预留结构 */
function tier1Fields(r: SearchableRelease): string[] {
  return [
    `${r.owner ?? ''}/${r.repo ?? ''}`,
    r.owner ?? '',
    r.repo ?? '',
    r.tag_name ?? '',
    r.release_name ?? '',
    r.source_description ?? '',
    r.ai_summary ?? '',
    // 视频源 body 即简介/标签串，是唯一内容载体（ai_summary 恒为空），纳入常规搜索
    isSummaryBodySource(r) ? (r.body ?? '') : '',
  ].map(s => s.toLowerCase())
}

/** Tier2 字段（GitHub / HF 的长正文 + 译文）。仅在深度搜索时构建。
 *  视频源已在 Tier1 覆盖，返回空串避免重复持有同一份文本。 */
function tier2Fields(r: SearchableRelease): string[] {
  if (isSummaryBodySource(r)) return ['']
  return [(r.body ?? ''), (r.body_translated ?? '')].map(s => s.toLowerCase())
}

// ── 缓存 ─────────────────────────────────────────────────────────
// 按数组引用缓存：App.vue 每次 loadReleases() 整体替换 releases.value，
// 故每次刷新都会重建。Tier1 真实规模约 6ms，无感。
// Tier2 不进 WeakMap —— 它必须由调用方显式构建与释放。
const tier1Cache = new WeakMap<readonly SearchableRelease[], string[][]>()

export function getSearchIndex(releases: readonly SearchableRelease[]): string[][] {
  let idx = tier1Cache.get(releases)
  if (!idx) {
    idx = releases.map(tier1Fields)
    tier1Cache.set(releases, idx)
  }
  return idx
}

/** 深度搜索用：构建 Tier2 索引（长正文 + 译文）。
 *  调用方负责在用完后丢弃引用（不缓存，交给 GC）。 */
export function buildBodyIndex(releases: readonly SearchableRelease[]): string[][] {
  return releases.map(tier2Fields)
}

/** 对单个 release 判断：所有 token 都必须命中 fields 中的某一个字段。 */
function matchesFields(fields: readonly string[], tokens: string[]): boolean {
  for (const t of tokens) {
    let hit = false
    for (const f of fields) {
      if (f.includes(t)) { hit = true; break }
    }
    if (!hit) return false
  }
  return true
}

// ── 对外 API ─────────────────────────────────────────────────────
// 两个入口：批量（走索引，供 ReleaseTab 用）与单条（供测试用）
// 单条入口无法复用批量索引，仅用于测试/低频场景。

/** 批量过滤：返回命中的下标序列，调用方再按下标取对象。
 *  这样 Tier1 索引只遍历一次，且与 releases 数组天然对齐。
 *  @param bodyIndex 传入 buildBodyIndex() 的结果即启用深度搜索 */
export function filterReleaseIndices(
  releases: readonly SearchableRelease[],
  query: string,
  bodyIndex?: readonly string[][] | null,
): number[] {
  const tokens = tokenizeQuery(query)
  if (tokens.length === 0) return releases.map((_, i) => i)

  const tier1 = getSearchIndex(releases)
  const out: number[] = []
  for (let i = 0; i < releases.length; i++) {
    if (matchesFields(tier1[i], tokens)) { out.push(i); continue }
    if (bodyIndex && matchesFields(bodyIndex[i], tokens)) out.push(i)
  }
  return out
}

/** 单条判断（供测试与既有调用方使用）。
 *  @param bodyFields 该条的 Tier2 字段数组；传入即启用深度搜索 */
export function releaseMatchesSearch(
  release: SearchableRelease,
  query: string,
  bodyFields?: readonly string[] | null,
): boolean {
  const tokens = tokenizeQuery(query)
  if (tokens.length === 0) return true
  if (matchesFields(tier1Fields(release), tokens)) return true
  return bodyFields ? matchesFields(bodyFields, tokens) : false
}

export function logLevelClass(level: string): string {
  switch (level) {
    case 'ERROR': return 'log-error'
    case 'WARN': return 'log-warn'
    default: return 'log-info'
  }
}

export function statusLabel(status: string, snoozeUntil?: string | null): string {
  if (isUnreadStatus(status, snoozeUntil)) return t('status.pending')
  if (status === 'snoozed') return t('status.snoozed')
  if (isReadStatus(status)) return t('status.viewed')
  return status
}

export function statusClass(status: string, snoozeUntil?: string | null): string {
  if (isUnreadStatus(status, snoozeUntil)) return 'status-unread'
  if (status === 'snoozed') return 'status-snoozed'
  if (isReadStatus(status)) return 'status-read'
  return 'status-unknown'
}

export function isUnreadStatus(status: string, snoozeUntil?: string | null): boolean {
  if (status === 'snoozed' && snoozeUntil) {
    const until = new Date(snoozeUntil).getTime()
    if (!isNaN(until) && until > Date.now()) {
      return false
    }
  }
  return status === 'pending' || status === 'snoozed'
}

export function isReadStatus(status: string): boolean {
  return status === 'clicked' || status === 'ignored'
}

/** skill 路径短名：去掉尾部分隔符，取最后一段；
 * 路径指向文件（如 …/commit/SKILL.md）时取所属目录名（skill 名），展示用。 */
export function skillShortName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '')
  const segs = trimmed.split(/[\\/]/)
  let seg = segs.pop()
  if (seg && segs.length > 0 && /\.[A-Za-z0-9]+$/.test(seg)) {
    // 末段是文件（带扩展名）：取上一段目录名，避免显示成 SKILL.md
    seg = segs.pop()
  }
  return seg && seg.length > 0 ? seg : trimmed
}
