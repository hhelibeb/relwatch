import { nextTick, onMounted, onUnmounted, ref, watch, type Ref } from 'vue'
import { registerFindHandler } from './contextMenuBus'
import { tokenizeQuery } from '../utils'
import {
  clearActiveHighlight,
  collectPhraseRanges,
  collectQueryRanges,
  registerHighlightRanges,
  searchHighlightSupported,
  setActiveHighlightRange,
  unregisterHighlightOwner,
} from '../utils/searchHighlight'

export interface UseElementFindOptions {
  container: Ref<HTMLElement | null>
  skipSelector?: string
  fallbackQuery?: () => string
  contentSource: () => string | null | undefined
  isStreaming?: () => boolean
  /** 顶层存在更高遮罩（如灯箱）时返回 true：仍消费 Ctrl+F 防止穿透到全局搜索，但不打开查找栏 */
  isBlocked?: () => boolean
  /**
   * 是否轮到自己接管 Ctrl+F（返回 false 则不消费，交由接管器栈的下一层）。
   * 常驻挂载但**不一定**是当前焦点的面板（如 Agent 工作区侧栏）靠它划定边界，
   * 避免霸占全局 Ctrl+F、把版本页搜索挤掉。
   */
  shouldHandle?: () => boolean
}

const MAX_PREFILL_SELECTION_LEN = 200
const SCROLL_COMFORT_PADDING = 16

function getRangeRect(range: Range): DOMRect | null {
  if (typeof range.getBoundingClientRect === 'function') {
    const rect = range.getBoundingClientRect()
    if (rect.width > 0 || rect.height > 0) return rect
  }
  if (typeof range.getClientRects === 'function') {
    const rects = range.getClientRects()
    for (let i = 0; i < rects.length; i++) {
      const r = rects[i]
      if (r.width > 0 || r.height > 0) return r
    }
  }
  return null
}

const SCROLLABLE_OVERFLOW = new Set(['auto', 'scroll', 'overlay'])

function isScrollableBox(el: HTMLElement): boolean {
  if (el.scrollHeight <= el.clientHeight + 1) return false
  if (typeof getComputedStyle !== 'function') return true
  return SCROLLABLE_OVERFLOW.has(getComputedStyle(el).overflowY)
}

/** 把命中区间滚进 el 的可视区（padding 为边缘留白；smooth 仅给最外层用）。 */
function revealWithin(el: HTMLElement, range: Range, padding: number, smooth: boolean): void {
  const rect = getRangeRect(range)
  const box = el.getBoundingClientRect()
  if (!rect || box.height <= 0) return
  if (rect.top >= box.top + padding && rect.bottom <= box.bottom - padding) return

  const topInBox = rect.top - box.top + el.scrollTop
  const targetTop = Math.max(0, topInBox - el.clientHeight / 3)
  if (smooth && typeof el.scrollTo === 'function') {
    el.scrollTo({ top: targetTop, behavior: 'smooth' })
  } else {
    el.scrollTop = targetTop
  }
}

function scrollRangeIntoView(container: HTMLElement | null, range: Range | null): void {
  if (!container || !range) return
  if (container.getBoundingClientRect().height <= 0) return
  revealWithin(container, range, SCROLL_COMFORT_PADDING, true)

  // 折叠块内层还有自己的滚动区（如超长工具输出的 max-height + overflow）：
  // 外层滚到位不等于命中在视野内
  const inner: HTMLElement[] = []
  let el: HTMLElement | null = range.startContainer instanceof Element
    ? (range.startContainer as HTMLElement)
    : range.startContainer.parentElement
  while (el && el !== container) {
    inner.push(el)
    el = el.parentElement
  }
  for (const node of inner) {
    if (isScrollableBox(node)) revealWithin(node, range, 0, false)
  }
}

export function useElementFind(options: UseElementFindOptions) {
  const open = ref(false)
  const query = ref('')
  const inputEl = ref<HTMLInputElement | null>(null)
  const matchCount = ref(0)
  const currentIndex = ref(-1)

  const highlightOwner = {}
  let ranges: Range[] = []
  let runToken = 0
  let scrollToken = 0
  let disposed = false
  let unregisterFind: (() => void) | null = null

  function selectionInsideContainer(): string {
    const el = options.container.value
    if (!el) return ''
    const sel = window.getSelection()
    if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return ''
    const anchor = sel.anchorNode
    if (!anchor || !el.contains(anchor)) return ''
    const text = sel.toString().trim()
    if (!text || text.includes('\n') || text.length > MAX_PREFILL_SELECTION_LEN) return ''
    return text
  }

  function revealCollapsedAncestors(range: Range): boolean {
    const container = options.container.value
    let el: Element | null = range.startContainer instanceof Element
      ? range.startContainer
      : range.startContainer.parentElement
    let opened = false
    while (el && el !== container) {
      if (el instanceof HTMLDetailsElement && !el.open) {
        el.open = true
        opened = true
      }
      el = el.parentElement
    }
    return opened
  }

  /**
   * 跳转前先展开命中所处的闭合 <details>：折叠内容不参与布局，rect 为空，
   * 不展开就是「计数在跳、画面不动」。
   */
  function revealAndScrollTo(range: Range | null) {
    if (!range) return
    if (!revealCollapsedAncestors(range)) {
      scrollRangeIntoView(options.container.value, range)
      return
    }
    // 展开是异步生效的 DOM 变更：延迟滚动可能被下一跳/关闭/改词超越，用 token 作废旧回调
    const token = ++scrollToken
    void nextTick(() => {
      if (disposed || token !== scrollToken) return
      scrollRangeIntoView(options.container.value, range)
    })
  }

  function syncHighlight(opts: { scroll?: boolean; preserveIndex?: boolean }) {
    if (disposed) return
    const el = options.container.value
    if (!el) return

    if (!open.value) {
      const fallbackTokens = tokenizeQuery(options.fallbackQuery?.() ?? '')
      if (fallbackTokens.length === 0 || !searchHighlightSupported()) {
        unregisterHighlightOwner(highlightOwner)
        return
      }
      registerHighlightRanges(highlightOwner, collectQueryRanges(el, fallbackTokens, options.skipSelector))
      return
    }

    const q = query.value
    if (!q.trim()) {
      ranges = []
      matchCount.value = 0
      currentIndex.value = -1
      unregisterHighlightOwner(highlightOwner)
      clearActiveHighlight(highlightOwner)
      return
    }

    const collected = collectPhraseRanges(el, q, options.skipSelector)
    ranges = collected
    matchCount.value = collected.length
    if (collected.length === 0) {
      currentIndex.value = -1
      unregisterHighlightOwner(highlightOwner)
      clearActiveHighlight(highlightOwner)
      return
    }
    if (opts.preserveIndex && currentIndex.value >= 0) {
      currentIndex.value = Math.min(currentIndex.value, collected.length - 1)
    } else {
      currentIndex.value = 0
    }
    const activeRange = collected[currentIndex.value]
    if (searchHighlightSupported()) {
      registerHighlightRanges(highlightOwner, collected)
    }
    setActiveHighlightRange(highlightOwner, activeRange)
    if (opts.scroll) {
      revealAndScrollTo(activeRange)
    }
  }

  function applyHighlight(opts: { scroll?: boolean; preserveIndex?: boolean } = {}) {
    const token = ++runToken
    if (!open.value) {
      ranges = []
      matchCount.value = 0
      currentIndex.value = -1
      clearActiveHighlight(highlightOwner)
    }
    if (options.container.value) {
      syncHighlight(opts)
      return
    }
    void nextTick(() => {
      if (disposed || token !== runToken) return
      syncHighlight(opts)
    })
  }

  function goTo(delta: number) {
    if (!open.value || ranges.length === 0) return
    const nextIdx = currentIndex.value < 0
      ? (delta >= 0 ? 0 : ranges.length - 1)
      : (currentIndex.value + delta + ranges.length) % ranges.length
    currentIndex.value = nextIdx
    const activeRange = ranges[nextIdx]
    setActiveHighlightRange(highlightOwner, activeRange)
    revealAndScrollTo(activeRange)
  }

  function next() {
    goTo(1)
  }

  function prev() {
    goTo(-1)
  }

  function focusInput() {
    void nextTick(() => {
      inputEl.value?.focus()
      inputEl.value?.select()
    })
  }

  function openFind() {
    const wasOpen = open.value
    const selected = selectionInsideContainer()
    if (selected) {
      query.value = selected
    } else if (!wasOpen && !query.value) {
      const fallback = options.fallbackQuery?.().trim() ?? ''
      if (fallback) query.value = fallback
    }
    open.value = true
    if (!wasOpen) {
      applyHighlight({ scroll: true, preserveIndex: false })
    }
    focusInput()
  }

  function closeFind() {
    if (!open.value) return
    open.value = false
    applyHighlight({ scroll: false })
  }

  function handleInputKeydown(e: KeyboardEvent) {
    if (e.isComposing) return
    if (e.key === 'Enter') {
      e.preventDefault()
      e.stopPropagation()
      if (e.shiftKey) prev()
      else next()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      closeFind()
    }
  }

  watch(query, () => {
    if (open.value) applyHighlight({ scroll: true, preserveIndex: false })
  })

  watch(options.contentSource, () => {
    const streaming = Boolean(options.isStreaming?.())
    applyHighlight({ scroll: !streaming, preserveIndex: streaming })
  }, { immediate: true, flush: 'post' })

  watch(() => options.fallbackQuery?.() ?? '', () => {
    if (!open.value) applyHighlight({ scroll: false })
  })

  onMounted(() => {
    unregisterFind = registerFindHandler(() => {
      if (options.shouldHandle && !options.shouldHandle()) return false
      if (options.isBlocked?.()) return true
      openFind()
      return true
    })
  })

  onUnmounted(() => {
    disposed = true
    unregisterFind?.()
    unregisterFind = null
    unregisterHighlightOwner(highlightOwner)
    clearActiveHighlight(highlightOwner)
  })

  return {
    open,
    query,
    inputEl,
    matchCount,
    currentIndex,
    openFind,
    closeFind,
    next,
    prev,
    handleInputKeydown,
  }
}
