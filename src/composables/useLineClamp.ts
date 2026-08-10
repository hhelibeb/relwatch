import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue'

/**
 * 行级截断测量引擎（两行满行 + 第三行精确截断 + 省略号 + 按钮避让）。
 *
 * 从 ReleaseItem 的摘要截断抽出的纯测量状态机：文本超过两行时，前两行满行、
 * 第三行在按钮左侧精确截断（自带省略号），按钮悬浮其右侧；不足两行时按钮独立
 * 成行。截断仅作用于显示层，tooltip/右键菜单仍用完整文本。
 *
 * 关键防御（历史回归点，勿删）：
 * - 隐藏克隆测量，克隆后必须重写为完整文本——cloneNode 会带走当前渲染的截断
 *   文本，后续 range.setEnd 越界抛 IndexSizeError，与 ResizeObserver 形成
 *   「截断 ↔ 重置」振荡。
 * - ResizeObserver 仅在宽度变化时重测（行数/截断点只受宽度影响），避免
 *   「截断状态切换 → 容器高度变化 → 触发观察 → 再切换」的自激振荡回路。
 */
export function useLineClamp(opts: {
  /** 待测量文本（响应式），如 () => previewContent.value */
  text: () => string | null
  /** 悬浮按钮选择器（相对文本元素的父级查找），如 '.release-expand-btn' */
  expandBtnSelector: string
  /** 按钮与文本的避让间距（px） */
  gap?: number
}) {
  const textRef = ref<HTMLElement | null>(null)
  const display = ref<string | null>(null)
  const hasThirdLine = ref(false)
  const EXPAND_GAP = opts.gap ?? 8
  let resizeObserver: ResizeObserver | null = null
  // 上次测量时的行宽：ResizeObserver 仅在宽度变化时重测（行数/截断点只受宽度影响），
  // 避免「截断状态切换 → 容器高度变化 → 触发观察 → 再切换」的自激振荡回路
  let lastWidth = -1

  function rangeLineCount(node: Node, end: number): number {
    const range = document.createRange()
    range.setStart(node, 0)
    range.setEnd(node, end)
    return range.getClientRects().length
  }

  // 第 3 行起始字符：text[0..i] 恰好排满两行的最小 i
  function findThirdLineStart(node: Node, text: string): number {
    let lo = 0
    let hi = text.length
    while (lo < hi) {
      const mid = (lo + hi) >> 1
      if (rangeLineCount(node, mid) >= 3) hi = mid
      else lo = mid + 1
    }
    return lo
  }

  function measureTextWidth(font: string, s: string): number {
    const probe = document.createElement('span')
    probe.style.cssText = `position:absolute;visibility:hidden;font:${font}`
    probe.textContent = s
    document.body.appendChild(probe)
    const w = probe.getBoundingClientRect().width
    document.body.removeChild(probe)
    return w
  }

  function measureLayout() {
    const el = textRef.value
    const text = opts.text()
    lastWidth = el?.clientWidth ?? -1
    if (!el || !text) {
      hasThirdLine.value = false
      display.value = null
      return
    }
    // 用隐藏克隆测量完整文本，不干扰显示。两个关键点：
    // 1) 克隆后必须重写为完整文本——cloneNode 会带走当前渲染的截断文本，
    //    后续 range.setEnd(node, text.length) 越界抛 IndexSizeError，状态被 catch 重置，
    //    与 ResizeObserver 形成「截断 ↔ 重置」振荡（窗口缩窄时按钮闪烁、跳动、盖住文字）。
    // 2) 克隆挂到同一父节点下，保证字体/行高等继承的排版上下文与真实元素一致。
    const clone = el.cloneNode(true) as HTMLElement
    clone.textContent = text
    clone.style.cssText = `position:absolute;visibility:hidden;left:0;top:0;width:${el.clientWidth}px;pointer-events:none`
    ;(el.parentElement ?? document.body).appendChild(clone)
    try {
      const node = clone.firstChild
      if (!node) {
        hasThirdLine.value = false
        display.value = null
        return
      }
      const totalLines = rangeLineCount(node, text.length)
      if (totalLines <= 2) {
        hasThirdLine.value = false
        display.value = null
        return
      }
      // 第三行可用宽度 = 行宽 - 按钮宽 - 间距
      const btn = el.parentElement?.querySelector<HTMLElement>(opts.expandBtnSelector)
      const btnWidth = btn?.getBoundingClientRect().width ?? 0
      const avail = el.clientWidth - btnWidth - EXPAND_GAP
      if (avail <= 0) {
        // 行宽连按钮都放不下：回退「按钮独立成行」，避免悬浮按钮盖住第三行文字
        hasThirdLine.value = false
        display.value = null
        return
      }
      hasThirdLine.value = true
      const start = findThirdLineStart(node, text)
      // 第三行剩余文字是否超出可用区：未超出则自然结束（省略号由 line-clamp 生成），无需截断。
      // 注意从 start-1 开始：start 是「text[0..i] 达到 3 行」的最小偏移，第三行首字符是 start-1
      const thirdRange = document.createRange()
      thirdRange.setStart(node, start - 1)
      thirdRange.setEnd(node, text.length)
      const thirdRects = thirdRange.getClientRects()
      const thirdWidth = thirdRects.length ? thirdRects[0].width : 0
      if (thirdWidth <= avail) {
        display.value = null
        return
      }
      // 二分找最大 end：slice(0, end) + '…' 的最后一行宽度 <= 可用宽
      const ellipsis = '…'
      const ellipsisW = measureTextWidth(getComputedStyle(el).font, ellipsis)
      let lo = start
      let hi = text.length
      while (lo < hi) {
        const mid = Math.ceil((lo + hi) / 2)
        const r = document.createRange()
        r.setStart(node, 0)
        r.setEnd(node, mid)
        const rects = r.getClientRects()
        const lastW = rects.length ? rects[rects.length - 1].width : 0
        if (lastW + ellipsisW <= avail) lo = mid
        else hi = mid - 1
      }
      let end = lo
      // 尽量在词边界截断（英文场景），中文不受影响
      if (end < text.length) {
        const ws = text.lastIndexOf(' ', end - 1)
        if (ws > start) end = ws
      }
      display.value = text.slice(0, end) + ellipsis
    } catch {
      // 测量环境不支持（如测试环境无布局引擎）时回退：按钮独立成行、显示原文
      hasThirdLine.value = false
      display.value = null
    } finally {
      clone.remove()
    }
  }

  function refresh() {
    void nextTick(measureLayout)
    // ResizeObserver 监听容器宽度变化（窗口缩放 → 行数/截断点变化）
    resizeObserver?.disconnect()
    resizeObserver = null
    const parent = textRef.value?.parentElement
    if (parent && typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(() => {
        // 仅宽度变化才重测：截断状态切换本身会改变容器高度，不过滤会形成振荡回路
        const w = textRef.value?.clientWidth ?? -1
        if (w === lastWidth) return
        measureLayout()
      })
      resizeObserver.observe(parent)
    }
  }

  watch(() => opts.text(), refresh)
  onMounted(refresh)
  onUnmounted(() => resizeObserver?.disconnect())

  return { textRef, display, hasThirdLine, refresh }
}
