import { tokenizeQuery } from '../utils'

export interface HighlightSegment {
  text: string
  hit: boolean
}

const HIGHLIGHT_NAME = 'search-hit'
const MAX_RANGES_PER_ROOT = 300
const MAX_TOTAL_RANGES = 2000

const owners = new Map<object, Range[]>()
let rebuildScheduled = false

export function searchHighlightSupported(): boolean {
  return typeof CSS !== 'undefined' && 'highlights' in CSS && typeof Highlight !== 'undefined'
}

/** 收集 root 内所有命中 token 的文本区间。skipSelector 内的文本（按钮/时间/状态等）不参与。 */
export function collectQueryRanges(root: HTMLElement, tokens: string[], skipSelector?: string): Range[] {
  const ranges: Range[] = []
  if (tokens.length === 0) return ranges
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  let node = walker.nextNode() as Text | null
  while (node && ranges.length < MAX_RANGES_PER_ROOT) {
    if (!skipSelector || !node.parentElement?.closest(skipSelector)) {
      const data = node.data.toLowerCase()
      for (const token of tokens) {
        let idx = data.indexOf(token)
        while (idx !== -1 && ranges.length < MAX_RANGES_PER_ROOT) {
          const range = document.createRange()
          range.setStart(node, idx)
          range.setEnd(node, idx + token.length)
          ranges.push(range)
          idx = data.indexOf(token, idx + token.length)
        }
      }
    }
    node = walker.nextNode() as Text | null
  }
  return ranges
}

/**
 * CSS Custom Highlight API 注册表：高亮不改动 DOM——摘要的行级截断测量
 * （useLineClamp）依赖单一文本节点，插 <mark> 会让 range.setEnd 越界。
 * 各组件以自身为 owner 登记区间，rAF 合帧重建全局 Highlight。
 */
export function registerHighlightRanges(owner: object, ranges: Range[]): void {
  if (ranges.length === 0) owners.delete(owner)
  else owners.set(owner, ranges)
  scheduleRebuild()
}

export function unregisterHighlightOwner(owner: object): void {
  if (owners.delete(owner)) scheduleRebuild()
}

function scheduleRebuild() {
  if (rebuildScheduled || !searchHighlightSupported()) return
  rebuildScheduled = true
  requestAnimationFrame(() => {
    rebuildScheduled = false
    const highlight = new Highlight()
    let count = 0
    for (const ranges of owners.values()) {
      for (const range of ranges) {
        if (count >= MAX_TOTAL_RANGES) break
        highlight.add(range)
        count++
      }
    }
    if (count === 0) CSS.highlights.delete(HIGHLIGHT_NAME)
    else CSS.highlights.set(HIGHLIGHT_NAME, highlight)
  })
}

/** 按命中词切分文本（大小写不敏感、重叠区间合并），供正文命中片段渲染 <mark>。 */
export function splitByTokens(text: string, tokens: string[]): HighlightSegment[] {
  if (tokens.length === 0 || !text) return [{ text, hit: false }]
  const lower = text.toLowerCase()
  const marks: { start: number; end: number }[] = []
  for (const token of tokens) {
    let idx = lower.indexOf(token)
    while (idx !== -1) {
      marks.push({ start: idx, end: idx + token.length })
      idx = lower.indexOf(token, idx + token.length)
    }
  }
  if (marks.length === 0) return [{ text, hit: false }]
  marks.sort((a, b) => a.start - b.start)
  const merged: { start: number; end: number }[] = []
  for (const m of marks) {
    const last = merged[merged.length - 1]
    if (last && m.start <= last.end) last.end = Math.max(last.end, m.end)
    else merged.push({ ...m })
  }
  const segments: HighlightSegment[] = []
  let pos = 0
  for (const m of merged) {
    if (m.start > pos) segments.push({ text: text.slice(pos, m.start), hit: false })
    segments.push({ text: text.slice(m.start, m.end), hit: true })
    pos = m.end
  }
  if (pos < text.length) segments.push({ text: text.slice(pos), hit: false })
  return segments
}

/**
 * 正文命中片段：卡片可见字段已含全部命中词时返回 null（高亮本身已可见）；
 * 否则按候选正文顺序取第一个命中的上下文。原文优先（保留大小写），
 * 深度索引的小写全文兜底（命中在 600 字预览之外时）。
 *
 * 多词查询优先定位「可见字段缺失的词」：已可见的词再高亮一遍说明不了
 * 这条为什么从正文里被搜出来（真正的新信息往往在窗口外）。
 * 正文里不含任何缺失词时跳过该候选，避免产出无信息量的片段。
 */
export function extractBodySnippet(
  visibleText: string,
  bodyTexts: readonly (string | null | undefined)[],
  query: string,
  before = 24,
  after = 72,
): HighlightSegment[] | null {
  const tokens = tokenizeQuery(query)
  if (tokens.length === 0) return null
  const visible = visibleText.toLowerCase()
  const missing = tokens.filter(tk => !visible.includes(tk))
  if (missing.length === 0) return null
  for (const body of bodyTexts) {
    if (!body) continue
    const lower = body.toLowerCase()
    let first = -1
    for (const tk of missing) {
      const idx = lower.indexOf(tk)
      if (idx !== -1 && (first === -1 || idx < first)) first = idx
    }
    if (first === -1) continue
    const start = Math.max(0, first - before)
    const end = Math.min(body.length, first + after)
    const snippet = (start > 0 ? '…' : '') + body.slice(start, end) + (end < body.length ? '…' : '')
    return splitByTokens(snippet, tokens)
  }
  return null
}