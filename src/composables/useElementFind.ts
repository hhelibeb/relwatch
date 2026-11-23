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

function scrollRangeIntoView(container: HTMLElement | null, range: Range | null): void {
  if (!container || !range) return
  const rect = getRangeRect(range)
  const containerRect = container.getBoundingClientRect()
  if (!rect || containerRect.height <= 0) return
  const visibleTop = containerRect.top + SCROLL_COMFORT_PADDING
  const visibleBottom = containerRect.bottom - SCROLL_COMFORT_PADDING
  if (rect.top >= visibleTop && rect.bottom <= visibleBottom) return

  const topInContainer = rect.top - containerRect.top + container.scrollTop
  const targetTop = Math.max(0, topInContainer - container.clientHeight / 3)
  if (typeof container.scrollTo === 'function') {
    container.scrollTo({ top: targetTop, behavior: 'smooth' })
  } else {
    container.scrollTop = targetTop
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
      scrollRangeIntoView(el, activeRange)
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
    scrollRangeIntoView(options.container.value, activeRange)
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
