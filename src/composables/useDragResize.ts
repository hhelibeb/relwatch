import { onMounted, onUnmounted, nextTick, type Ref } from 'vue'

export type ResizeDir = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw'

interface DragResizeOptions {
  minWidth?: number
  minHeight?: number
  // 传入后把位置/尺寸持久化到 localStorage，下次挂载自动恢复（键名遵循 relwatch.* 约定）
  persistKey?: string
}

// 持久化形状：x/y 为期望的 left/top；w/h 仅在用户显式调整过尺寸时存在，
// 否则保留 CSS 默认的响应式宽度（min(760px, 100%)），不冻结默认布局
interface PersistedRect {
  x: number
  y: number
  w?: number
  h?: number
}

const RESIZE_CURSORS: Record<ResizeDir, string> = {
  n: 'ns-resize',
  s: 'ns-resize',
  e: 'ew-resize',
  w: 'ew-resize',
  ne: 'nesw-resize',
  sw: 'nesw-resize',
  nw: 'nwse-resize',
  se: 'nwse-resize',
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(Math.max(v, min), Math.max(min, max))
}

function isFiniteNumber(v: unknown): v is number {
  return typeof v === 'number' && Number.isFinite(v)
}

// 通用弹窗拖动 + 八向调整大小。
// 布局保持 flex 居中不变：拖动只叠加 transform 偏移，避免切换定位方式造成跳动；
// 调整大小时写入内联 width/height，并按手柄方向修正偏移，使对侧边缘保持不动
// （纯横向/纵向手柄的另一轴无需修正：居中基座 + 不变偏移 = 中心自动保持）。
// 内容驱动的自动高度变化（如切换摘要/译文）：补偿偏移使顶边（标题）位置固定，
// 窗口只从底部伸缩；底部越出视口时整体上移钳回（锚定让位于「完整可见」约束）。
export function useDragResize(target: Ref<HTMLElement | null>, options: DragResizeOptions = {}) {
  const minWidth = options.minWidth ?? 360
  const minHeight = options.minHeight ?? 240
  const persistKey = options.persistKey

  let offsetX = 0
  let offsetY = 0
  let sized = false // 用户是否显式调整过尺寸（决定是否持久化/恢复 w/h）
  let endActiveSession: (() => void) | null = null
  // 最近一次已知高度：手动 resize 写入尺寸时同步记账，
  // ResizeObserver 回调中据此区分「内容驱动」与「手动」的高度变化
  let lastHeight = 0
  let sizeObserver: ResizeObserver | null = null

  function applyTransform() {
    const el = target.value
    if (el) el.style.transform = `translate(${offsetX}px, ${offsetY}px)`
  }

  function endSession() {
    if (!endActiveSession) return
    endActiveSession()
    endActiveSession = null
  }

  // 一次拖动/调整会话：window 级 move/up 监听 + 文本选择与光标锁定，up 时统一收尾
  function beginSession(onMove: (e: PointerEvent) => void, onEnd: () => void, cursor?: string) {
    endSession()
    const prevUserSelect = document.body.style.userSelect
    const prevCursor = document.body.style.cursor
    document.body.style.userSelect = 'none'
    if (cursor) document.body.style.cursor = cursor

    const move = (e: PointerEvent) => {
      // 兜底：指针在窗口外松开时收不到 pointerup，按键已抬起则提前结束
      if ((e.buttons & 1) === 0) {
        up()
        return
      }
      onMove(e)
    }
    const up = () => {
      endSession()
      onEnd()
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)

    endActiveSession = () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
      document.body.style.userSelect = prevUserSelect
      document.body.style.cursor = prevCursor
    }
  }

  function capturePointer(e: PointerEvent) {
    const el = (e.currentTarget ?? e.target) as HTMLElement | null
    try {
      el?.setPointerCapture?.(e.pointerId)
    } catch {
      // jsdom 等环境不支持指针捕获，静默忽略
    }
  }

  function persist() {
    const el = target.value
    if (!el || !persistKey) return
    const rect = el.getBoundingClientRect()
    const data: PersistedRect = { x: rect.left, y: rect.top }
    if (sized) {
      data.w = rect.width
      data.h = rect.height
    }
    try {
      window.localStorage.setItem(persistKey, JSON.stringify(data))
    } catch {
      // 隐私模式等写入失败，静默忽略
    }
  }

  // 视口内允许的位置范围（弹窗始终完整留在窗口内）
  function clampedPosition(rect: { left: number; top: number; width: number; height: number }) {
    return {
      left: clamp(rect.left, 0, Math.max(0, window.innerWidth - rect.width)),
      top: clamp(rect.top, 0, Math.max(0, window.innerHeight - rect.height)),
    }
  }

  // 从持久化恢复：尺寸（若有）→ 按当前视口钳制位置 → 换算回居中基座上的偏移
  function restore() {
    const el = target.value
    if (!el || !persistKey) return
    let data: PersistedRect
    try {
      data = JSON.parse(window.localStorage.getItem(persistKey) ?? 'null') as PersistedRect
    } catch {
      return
    }
    if (!data || !isFiniteNumber(data.x) || !isFiniteNumber(data.y)) return
    if (isFiniteNumber(data.w) && isFiniteNumber(data.h)) {
      sized = true
      el.style.maxHeight = 'none'
      el.style.width = `${clamp(data.w, minWidth, window.innerWidth)}px`
      el.style.height = `${clamp(data.h, minHeight, window.innerHeight)}px`
    }
    const rect = el.getBoundingClientRect()
    const { left, top } = clampedPosition({ left: data.x, top: data.y, width: rect.width, height: rect.height })
    offsetX = left - (window.innerWidth - rect.width) / 2
    offsetY = top - (window.innerHeight - rect.height) / 2
    applyTransform()
  }

  // 窗口尺寸变化兜底：缩回视口外的弹窗，超出视口的显式尺寸同步收缩
  function clampIntoViewport() {
    const el = target.value
    if (!el) return
    let rect = el.getBoundingClientRect()
    if (sized) {
      const w = Math.min(rect.width, window.innerWidth)
      const h = Math.min(rect.height, window.innerHeight)
      if (w !== rect.width) el.style.width = `${w}px`
      if (h !== rect.height) el.style.height = `${h}px`
      rect = el.getBoundingClientRect()
      lastHeight = rect.height // 同步记账，避免被 ResizeObserver 当作内容驱动变化
    }
    const { left, top } = clampedPosition(rect)
    offsetX += left - rect.left
    offsetY += top - rect.top
    applyTransform()
  }

  function startDrag(e: PointerEvent) {
    const el = target.value
    if (!el || e.button !== 0) return
    // 头部内的按钮（关闭等）不触发拖动
    if ((e.target as HTMLElement | null)?.closest('button, a, input, textarea, select')) return
    e.preventDefault()
    capturePointer(e)
    let lastX = e.clientX
    let lastY = e.clientY
    beginSession((ev) => {
      const rect = el.getBoundingClientRect()
      const dx = ev.clientX - lastX
      const dy = ev.clientY - lastY
      lastX = ev.clientX
      lastY = ev.clientY
      const { left, top } = clampedPosition({
        left: rect.left + dx,
        top: rect.top + dy,
        width: rect.width,
        height: rect.height,
      })
      offsetX += left - rect.left
      offsetY += top - rect.top
      applyTransform()
    }, persist, 'move')
  }

  // 内容变化（切换摘要/译文、版本导航等）导致弹窗自动伸缩时，按高度差补偿偏移，
  // 保持顶边（标题）位置不变，窗口只从底部伸缩；手动 resize 的高度已记账，不会进入补偿
  function anchorTopOnAutoResize() {
    const el = target.value
    if (!el) return
    const rect = el.getBoundingClientRect()
    const dh = rect.height - lastHeight
    lastHeight = rect.height
    if (dh === 0) return
    // 视觉 top = (视口高 - h)/2 + offsetY，h 变化 dh 后补偿 dh/2 即可还原顶边位置
    offsetY += dh / 2
    applyTransform()
    // 内容变长时底部可能越出视口：整体上移钳回（顶边锚定让位于「完整可见」约束）
    const after = el.getBoundingClientRect()
    const { left, top } = clampedPosition(after)
    if (left !== after.left || top !== after.top) {
      offsetX += left - after.left
      offsetY += top - after.top
      applyTransform()
    }
  }

  function startResize(e: PointerEvent, dir: ResizeDir) {
    const el = target.value
    if (!el || e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()
    capturePointer(e)
    sized = true
    // 解除 CSS max-height 上限，尺寸完全交由 JS 约束（已钳制在视口内）
    el.style.maxHeight = 'none'
    // 解除上限后高度可能立即跳变（内容超出原上限）：同步记账，避免误触发顶边锚定
    lastHeight = el.getBoundingClientRect().height
    const hasW = dir.includes('w')
    const hasE = dir.includes('e')
    const hasN = dir.includes('n')
    const hasS = dir.includes('s')
    beginSession((ev) => {
      const rect = el.getBoundingClientRect()
      let w = rect.width
      let h = rect.height
      // 上限取「手柄方向上到视口边缘的剩余空间」，保证调整后仍完整在视口内
      if (hasE) w = clamp(ev.clientX - rect.left, minWidth, window.innerWidth - rect.left)
      if (hasW) w = clamp(rect.right - ev.clientX, minWidth, rect.right)
      if (hasS) h = clamp(ev.clientY - rect.top, minHeight, window.innerHeight - rect.top)
      if (hasN) h = clamp(rect.bottom - ev.clientY, minHeight, rect.bottom)
      // 修正偏移使对侧边缘固定（居中基座会随尺寸变化而移动）
      const baseLeft = (window.innerWidth - w) / 2
      const baseTop = (window.innerHeight - h) / 2
      if (hasE) offsetX = rect.left - baseLeft
      if (hasW) offsetX = rect.right - w - baseLeft
      if (hasS) offsetY = rect.top - baseTop
      if (hasN) offsetY = rect.bottom - h - baseTop
      el.style.width = `${w}px`
      el.style.height = `${h}px`
      lastHeight = h // 手动 resize 的高度同步记账，ResizeObserver 不再补偿
      applyTransform()
    }, persist, RESIZE_CURSORS[dir])
  }

  onMounted(() => {
    nextTick(() => {
      restore()
      const el = target.value
      if (el && typeof ResizeObserver !== 'undefined') {
        lastHeight = el.getBoundingClientRect().height
        sizeObserver = new ResizeObserver(anchorTopOnAutoResize)
        sizeObserver.observe(el)
      }
    })
    window.addEventListener('resize', clampIntoViewport)
  })
  onUnmounted(() => {
    endSession()
    sizeObserver?.disconnect()
    sizeObserver = null
    window.removeEventListener('resize', clampIntoViewport)
  })

  return {
    startDrag,
    startResize,
  }
}
