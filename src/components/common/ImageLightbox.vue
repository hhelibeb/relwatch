<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { t } from '../../i18n'
import { openReleaseUrl } from '../../api/client'
import { useOverlayEscape } from '../../composables/useOverlayEscape'

const props = defineProps<{ src: string; originalUrl: string }>()
const emit = defineEmits<{ close: [] }>()

const stageEl = ref<HTMLElement | null>(null)
const scale = ref(1)
const rotation = ref(0)
const offset = ref({ x: 0, y: 0 })
const loaded = ref(false)
const failed = ref(false)
const panning = ref(false)

// 组件整体被父级 v-if 挂载/卸载，Transition 看不到子级切换——
// 用内部 visible 驱动进出场：关闭时先播离场动画，再 emit 让父级卸载。
// 240ms 须覆盖 --dur-overlay（220ms）+ 余量，与 App.vue Toast 同口径。
const visible = ref(false)
let closeTimer: ReturnType<typeof setTimeout> | null = null

function requestClose() {
  if (closeTimer) return
  visible.value = false
  closeTimer = setTimeout(() => emit('close'), 240)
}

const MIN_SCALE = 0.2
const MAX_SCALE = 8

const percentLabel = computed(() => `${Math.round(scale.value * 100)}%`)
const imgStyle = computed(() => ({
  transform: `translate(${offset.value.x}px, ${offset.value.y}px) rotate(${rotation.value}deg) scale(${scale.value})`,
}))

function clampScale(v: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, v))
}

/** 以视口某点为锚缩放：保持指针下的图像点不动；无锚点时以中心缩放 */
function zoomAt(next: number, px?: number, py?: number) {
  const s = clampScale(next)
  const stage = stageEl.value
  if (stage && px !== undefined && py !== undefined && s !== scale.value) {
    const rect = stage.getBoundingClientRect()
    const cx = px - rect.left - rect.width / 2
    const cy = py - rect.top - rect.height / 2
    const k = s / scale.value
    offset.value = { x: cx - (cx - offset.value.x) * k, y: cy - (cy - offset.value.y) * k }
  }
  scale.value = s
  if (s === 1) offset.value = { x: 0, y: 0 }
}

function zoomStep(factor: number) {
  zoomAt(scale.value * factor)
}

function rotateStep() {
  rotation.value = (rotation.value + 90) % 360
}

function resetView() {
  scale.value = 1
  rotation.value = 0
  offset.value = { x: 0, y: 0 }
}

function onWheel(e: WheelEvent) {
  e.preventDefault()
  zoomAt(scale.value * (e.deltaY < 0 ? 1.15 : 1 / 1.15), e.clientX, e.clientY)
}

let panStart: { x: number; y: number; ox: number; oy: number } | null = null
let moved = false
let downOnImage = false

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  const target = e.target as HTMLElement
  if (target.closest('.lightbox-toolbar')) return
  downOnImage = target.tagName === 'IMG'
  panStart = { x: e.clientX, y: e.clientY, ox: offset.value.x, oy: offset.value.y }
  moved = false
  // 全屏 stage 内拖拽不依赖 capture 也成立；不支持的环境（测试/旧 WebView）跳过
  try {
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  } catch {
    // ignore
  }
}

function onPointerMove(e: PointerEvent) {
  if (!panStart) return
  const dx = e.clientX - panStart.x
  const dy = e.clientY - panStart.y
  if (Math.abs(dx) + Math.abs(dy) > 3) {
    moved = true
    panning.value = true
  }
  if (moved) offset.value = { x: panStart.ox + dx, y: panStart.oy + dy }
}

function onPointerUp() {
  if (!panStart) return
  panStart = null
  panning.value = false
  // 点击空白关闭；点图片本身不关（双击缩放）。pointer capture 会把 up 的
  // target 重定向到 stage，故用 down 时记录的目标判定
  if (!moved && !downOnImage) requestClose()
}

function onDblClick(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('.lightbox-toolbar')) return
  if (scale.value === 1) zoomAt(2.5, e.clientX, e.clientY)
  // 缩回只复位缩放：双击是“在缩放之间切换”，顺带把角度归零会让旋转过的图片突然弹回
  else zoomAt(1)
}

// 捕获期拦截：Esc 先关灯箱，方向键不穿透到详情弹窗的版本导航。
// active 跟随 visible：离场动画（240ms）期间已不是可见浮层，不该再吃按键、
// 也不该让 hasActiveOverlay() 继续为真（那会屏蔽背后的列表快捷键）
useOverlayEscape({
  onEscape: requestClose,
  active: () => visible.value,
  capture: true,
  preventDefault: true,
  stopPropagation: true,
  onKeydown: (e) => {
    if (e.key.startsWith('Arrow')) {
      e.preventDefault()
      e.stopPropagation()
      return
    }
    // 带修饰键的组合（如 App 全局 Ctrl+R 刷新轮询）不接管
    if (e.ctrlKey || e.metaKey || e.altKey) return
    if (e.key.toLowerCase() !== 'r') return
    e.preventDefault()
    e.stopPropagation()
    rotateStep()
  },
})

function handleOpenOriginal() {
  void openReleaseUrl(props.originalUrl)
}

onMounted(() => {
  visible.value = true
})
onUnmounted(() => {
  if (closeTimer) clearTimeout(closeTimer)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="lightbox">
      <div
        v-if="visible"
        ref="stageEl"
        class="lightbox-stage"
        :class="{ panning }"
        role="dialog"
        aria-modal="true"
        :aria-label="t('lightbox.title')"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerUp"
        @dblclick="onDblClick"
        @wheel.prevent="onWheel"
      >
        <img
          v-if="!failed"
          class="lightbox-img"
          :class="{ loaded, rotated: rotation % 180 !== 0 }"
          :src="props.src"
          :style="imgStyle"
          alt=""
          draggable="false"
          @load="loaded = true"
          @error="failed = true"
        />
        <div v-else class="lightbox-error">{{ t('lightbox.load_failed') }}</div>
        <div class="lightbox-toolbar" @pointerdown.stop>
          <button type="button" class="lightbox-btn" :title="t('lightbox.zoom_out')" :disabled="failed || scale <= MIN_SCALE" @click="zoomStep(1 / 1.25)">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M3.5 8h9"/></svg>
          </button>
          <span class="lightbox-percent">{{ percentLabel }}</span>
          <button type="button" class="lightbox-btn" :title="t('lightbox.zoom_in')" :disabled="failed || scale >= MAX_SCALE" @click="zoomStep(1.25)">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M8 3.5v9M3.5 8h9"/></svg>
          </button>
          <button type="button" class="lightbox-btn" :title="t('lightbox.zoom_reset')" :disabled="failed" @click="resetView">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2.5H2.5V6M10 2.5h3.5V6M6 13.5H2.5V10M10 13.5h3.5V10"/></svg>
          </button>
          <button type="button" class="lightbox-btn" :title="t('lightbox.rotate')" :disabled="failed" @click="rotateStep">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M13.5 8a5.5 5.5 0 1 1-2.2-4.4"/><path d="M13.5 2v4h-4"/></svg>
          </button>
          <span class="lightbox-divider"></span>
          <button type="button" class="lightbox-btn" :title="t('lightbox.open_original')" @click="handleOpenOriginal">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h-3v9h9v-3M9.5 2.5h4v4M13.5 2.5l-6.5 6.5"/></svg>
          </button>
          <button type="button" class="lightbox-btn" :title="t('release.detail_close')" @click="requestClose">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.lightbox-stage {
  position: fixed;
  inset: 0;
  z-index: var(--z-popover-modal);
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.82);
  cursor: grab;
  touch-action: none;
  overflow: hidden;
}

.lightbox-stage.panning {
  cursor: grabbing;
}

.lightbox-img {
  max-width: 92vw;
  max-height: 88vh;
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  user-select: none;
  -webkit-user-drag: none;
  opacity: 0;
  transition: opacity var(--dur-overlay) var(--ease-out);
  will-change: transform;
}

.lightbox-img.loaded {
  opacity: 1;
}

/* 旋转 90°/270° 后布局盒长宽互换（rotate 不改布局尺寸），
   适配上限必须对调，否则横向长截图转完会溢出视口被裁 */
.lightbox-img.rotated {
  max-width: 88vh;
  max-height: 92vw;
}

.lightbox-error {
  padding: 12px 16px;
  background: var(--surface);
  border-radius: var(--radius);
  color: var(--text);
  font-size: var(--fs-body);
}

.lightbox-toolbar {
  position: absolute;
  bottom: 20px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 6px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  box-shadow: var(--shadow-lg);
  cursor: default;
}

.lightbox-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: none;
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out);
}

.lightbox-btn:hover:not(:disabled) {
  background: var(--hover-surface);
  color: var(--text);
}

.lightbox-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.lightbox-btn svg {
  width: 14px;
  height: 14px;
}

.lightbox-percent {
  min-width: 44px;
  text-align: center;
  font-family: var(--mono-font);
  font-size: var(--fs-meta);
  color: var(--text);
}

.lightbox-divider {
  width: 1px;
  height: 16px;
  margin: 0 2px;
  background: var(--border);
}

.lightbox-enter-active,
.lightbox-leave-active {
  transition: opacity var(--dur-overlay) var(--ease-out);
}

.lightbox-enter-from,
.lightbox-leave-to {
  opacity: 0;
}
</style>
