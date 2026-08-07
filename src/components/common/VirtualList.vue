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
