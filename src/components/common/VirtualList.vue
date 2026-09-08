<script setup lang="ts" generic="T">
import { ref, shallowRef, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'

// 虚拟列表：大列表（版本记录全量历史可达上千条）只渲染可视区附近的行。
// - 监听最近的可滚动祖先（应用主滚动容器），按 scrollTop 计算可视区间
// - 行高可变：可见行渲染后测量一次并缓存，之后滚动复用
// - 列表小于 virtualizeThreshold 时直接全量渲染，规避虚拟化边界问题（测试/小数据）
const props = withDefaults(defineProps<{
  items: T[]
  itemKey: (item: T, index: number) => string | number
  estimatedHeight?: number
  overscan?: number
  gap?: number
  virtualizeThreshold?: number
}>(), {
  estimatedHeight: 180,
  overscan: 400,
  gap: 8,
  virtualizeThreshold: 100,
})

/** 定位时的额外留白：目标行与顶部 sticky 栏之间留一点呼吸空间（px）。 */
const EDGE_PADDING = 12

const containerEl = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewport = ref(0)
// 行高缓存：shallowRef 保证测量写回后触发 measured/visibleRows 重算，行重排收敛
const heights = shallowRef(new Map<string, number>())

let scrollParent: HTMLElement | null = null
let ro: ResizeObserver | null = null
let roSelf: ResizeObserver | null = null

const virtualizing = computed(() => props.items.length > props.virtualizeThreshold)

function keyOf(item: T, index: number): string {
  return String(props.itemKey(item, index))
}

function itemHeight(key: string): number {
  return (heights.value.get(key) ?? props.estimatedHeight) + props.gap
}

const measured = computed(() => {
  const offsets: number[] = []
  let acc = 0
  for (let i = 0; i < props.items.length; i++) {
    offsets.push(acc)
    acc += itemHeight(keyOf(props.items[i], i))
  }
  return { offsets, total: acc }
})

function lowerBound(offsets: number[], target: number): number {
  let lo = 0
  let hi = offsets.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (offsets[mid] + props.estimatedHeight <= target) lo = mid + 1
    else hi = mid
  }
  return lo
}

const visibleRows = computed(() => {
  const items = props.items
  if (!virtualizing.value || items.length === 0) return []
  const { offsets } = measured.value
  const start = scrollTop.value - props.overscan
  const end = scrollTop.value + viewport.value + props.overscan
  const rows: { item: T; index: number; top: number }[] = []
  for (let i = lowerBound(offsets, start); i < items.length; i++) {
    if (offsets[i] > end) break
    rows.push({ item: items[i], index: i, top: offsets[i] })
  }
  return rows
})

// 可见行渲染后经 :ref 回调测量真实高度（每个 key 只测一次）。
// 行挂载时同步读取 offsetHeight 并写回 heights，触发 measured/visibleRows 重排收敛；
// 行被滚动卸载再回来时复用缓存，无需重复测量。
function recordRow(row: { item: T; index: number }, el: unknown) {
  if (!el) return
  const key = keyOf(row.item, row.index)
  const h = (el as HTMLElement).offsetHeight
  if (h > 0 && heights.value.get(key) !== h) {
    heights.value = new Map(heights.value).set(key, h)
  }
}

// v-show 场景：容器被隐藏（display:none）时挂载的行 offsetHeight 恒为 0，
// 行高测量被跳过，所有行回退 estimatedHeight 摆位（间距虚大）。
// 从隐藏变为可见（尺寸 0→非0）时 ResizeObserver 触发本回调，
// 重新测量已挂载的行并写回缓存，布局随 measured/visibleRows 重算收敛。
function remeasureVisibleRows() {
  const root = containerEl.value
  if (!root || root.offsetHeight === 0) return
  for (const el of root.querySelectorAll<HTMLElement>('.virtual-item')) {
    const key = el.dataset.vkey
    if (!key) continue
    const h = el.offsetHeight
    if (h > 0 && heights.value.get(key) !== h) {
      heights.value = new Map(heights.value).set(key, h)
    }
  }
}

function findScrollParent(el: HTMLElement): HTMLElement | null {
  let cur: HTMLElement | null = el.parentElement
  while (cur) {
    const oy = window.getComputedStyle(cur).overflowY
    if (oy === 'auto' || oy === 'scroll' || oy === 'overlay') return cur
    cur = cur.parentElement
  }
  return null
}

function onScroll() {
  if (scrollParent) scrollTop.value = scrollParent.scrollTop
}

async function setupScroll() {
  await nextTick()
  const root = containerEl.value
  if (!root || scrollParent) return
  const parent = findScrollParent(root)
  if (!parent) return
  scrollParent = parent
  viewport.value = parent.clientHeight
  scrollTop.value = parent.scrollTop
  parent.addEventListener('scroll', onScroll, { passive: true })
  if (typeof ResizeObserver !== 'undefined') {
    ro = new ResizeObserver(() => {
      viewport.value = parent.clientHeight
    })
    ro.observe(parent)
    // 观察容器自身：v-show 隐藏→可见时尺寸 0→非0 触发，重新测量行高
    roSelf = new ResizeObserver(remeasureVisibleRows)
    roSelf.observe(root)
  }
}

function teardownScroll() {
  if (scrollParent) {
    scrollParent.removeEventListener('scroll', onScroll)
    scrollParent = null
  }
  ro?.disconnect()
  ro = null
  roSelf?.disconnect()
  roSelf = null
}

/** 滚动容器内吸附在顶部的元素（sticky 搜索/筛选栏）在视口顶部占住的高度。
 *  定位目标行时必须为它们留位，否则行顶部会被 sticky 元素遮住。
 *  `top` 可能为负（滚动后收起，如 `top: -16px`），此时实际遮挡 = 高度 + top。 */
function measureStickyTop(parent: HTMLElement): number {
  let inset = 0
  for (const el of parent.querySelectorAll<HTMLElement>('*')) {
    const cs = window.getComputedStyle(el)
    if (cs.position !== 'sticky' || cs.display === 'none') continue
    const top = Number.parseFloat(cs.top)
    const occupy = el.offsetHeight + (Number.isFinite(top) ? top : 0)
    if (occupy > inset) inset = occupy
  }
  return inset
}

/** 滚动使指定下标行可见（供“通知定位”等外部按索引定位）。
 *  虚拟化场景：目标行已挂载则直接精确校正；未挂载则先按估算把它滚进渲染窗口，
 *  等一帧行渲染后再精确校正（行高可能变化）。非虚拟化直接 `scrollIntoView`。
 *  两种场景都为顶部 sticky 元素预留空间，避免目标行被搜索/筛选栏遮住。 */
function scrollToIndex(index: number) {
  if (index < 0 || index >= props.items.length) return
  if (!virtualizing.value) {
    const items = containerEl.value?.querySelectorAll<HTMLElement>('.virtual-item, .virtual-list-plain > *')
    const el = items?.[index]
    if (!el) return
    // 原生 scrollIntoView 不感知 sticky 遮挡：用 scroll-margin-top 显式留出顶部安全区。
    // 非虚拟化时未建立滚动监听，这里临时查找一次滚动容器（纯函数，无副作用）。
    const parent = scrollParent ?? (containerEl.value ? findScrollParent(containerEl.value) : null)
    const inset = parent ? measureStickyTop(parent) : 0
    if (inset > 0) el.style.scrollMarginTop = `${inset + EDGE_PADDING}px`
    el.scrollIntoView({ block: 'nearest' })
    return
  }
  const parent = scrollParent
  const root = containerEl.value
  if (!parent || !root) return
  const targetKey = keyOf(props.items[index], index)
  const inset = measureStickyTop(parent)
  const findRowEl = () => {
    const cur = containerEl.value
    if (!cur) return null
    for (const el of cur.querySelectorAll<HTMLElement>('.virtual-item')) {
      if (el.dataset.vkey === targetKey) return el
    }
    return null
  }
  /** 精确校正：把行顶部对齐到「sticky 安全区之下」，并在剩余可用区内居中
   *  （行高于可用区时贴安全区顶部，保证至少顶部完整可见）。
   *  增量按“行相对滚动容器视口的 y”计算——不能用“行相对列表容器的偏移”，
   *  两者相差列表容器顶部在视口中的位置，会导致越往后的行滚得越过头。 */
  const alignRow = (): boolean => {
    const el = findRowEl()
    if (!el) return false
    const free = Math.max(0, parent.clientHeight - inset)
    const pad = el.offsetHeight > free ? 0 : Math.max(0, (free - el.offsetHeight) / 2)
    const currentY = el.getBoundingClientRect().top - parent.getBoundingClientRect().top
    // 夹住下界：顶部目标行可能在无滚动空间时算出负值（浏览器也会夹，但显式更明确）
    parent.scrollTop = Math.max(0, parent.scrollTop + currentY - inset - pad)
    return true
  }
  // 已挂载（可能在视口外）：直接校正
  if (alignRow()) return
  // 未挂载：先按估算把它滚进渲染窗口，再等帧校正。
  // 列表容器顶部在“滚动内容坐标”中的位置 = 视口偏移 + 当前 scrollTop。
  const containerInContent =
    root.getBoundingClientRect().top - parent.getBoundingClientRect().top + parent.scrollTop
  const coarse = (off: number) => {
    parent.scrollTop = Math.max(0, containerInContent + off - inset - EDGE_PADDING)
  }
  coarse(measured.value.offsets[index] ?? 0)
  // 行高在渲染过程中逐步被测出，估算值随之收敛：每帧重试校正，失败则按最新
  // 测量值重新粗定位，直到命中或达到帧数上限（避免无限 rAF）。
  let tries = 0
  const step = () => {
    // 组件已卸载/列表已重建：停止重试（scrollParent 在 teardown 时置空）
    if (!containerEl.value || parent !== scrollParent) return
    if (alignRow()) return
    coarse(measured.value.offsets[index] ?? 0)
    if (++tries < 4) requestAnimationFrame(step)
  }
  requestAnimationFrame(step)
}

defineExpose({ scrollToIndex })

watch(virtualizing, (v) => {
  if (v) setupScroll()
  else teardownScroll()
})

onMounted(() => {
  if (virtualizing.value) setupScroll()
})

onUnmounted(teardownScroll)
</script>

<template>
  <div v-if="virtualizing" ref="containerEl" class="virtual-list" :style="{ height: measured.total + 'px' }">
    <div
      v-for="row in visibleRows"
      :key="keyOf(row.item, row.index)"
      :data-vkey="keyOf(row.item, row.index)"
      :ref="(el) => recordRow(row, el)"
      class="virtual-item"
      :style="{ top: row.top + 'px' }"
    >
      <slot :item="row.item" :index="row.index" />
    </div>
  </div>
  <div v-else ref="containerEl" class="virtual-list-plain" :style="{ gap: props.gap + 'px' }">
    <template v-for="(item, i) in props.items" :key="keyOf(item, i)">
      <slot :item="item" :index="i" />
    </template>
  </div>
</template>

<style scoped>
.virtual-list {
  position: relative;
}

.virtual-item {
  position: absolute;
  left: 0;
  right: 0;
}

.virtual-list-plain {
  display: flex;
  flex-direction: column;
}
</style>
