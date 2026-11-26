import { computed, onUnmounted, ref, type Ref } from 'vue'

const TOAST_SWAP_DELAY = 240
const TOAST_MIN_DURATION = 2600
const TOAST_MAX_DURATION = 6000
const TOAST_GAP = 20

export function toastDuration(message: string): number {
  const n = [...message].length
  return Math.min(TOAST_MAX_DURATION, Math.max(TOAST_MIN_DURATION, 1200 + n * 175))
}

export interface UseToastOptions {
  agentPanelVisible: Ref<boolean>
  agentPanelWidth: Ref<number>
  dividerWidth: number
}

export function useToast(options: UseToastOptions) {
  const toastMessage = ref('')
  const toastVisible = ref(false)
  let toastTimer: ReturnType<typeof setTimeout> | null = null
  const toastQueue: string[] = []
  let toastSwapTimer: ReturnType<typeof setTimeout> | null = null
  let toastHovered = false

  const toastStyle = computed(() => {
    const right = options.agentPanelVisible.value
      ? options.agentPanelWidth.value + options.dividerWidth + TOAST_GAP
      : TOAST_GAP
    return {
      right: `${right}px`,
      maxWidth: `min(360px, calc(100vw - ${right + TOAST_GAP}px))`,
    }
  })

  function clearToastTimer() {
    if (toastTimer) {
      clearTimeout(toastTimer)
      toastTimer = null
    }
  }

  function startToastTimer() {
    clearToastTimer()
    toastTimer = setTimeout(() => {
      toastTimer = null
      dismissCurrentToast()
    }, toastDuration(toastMessage.value))
  }

  function showNextToast() {
    const next = toastQueue.shift()
    if (next === undefined) return
    toastMessage.value = next
    toastVisible.value = true
    if (!toastHovered) startToastTimer()
  }

  function dismissCurrentToast() {
    if (!toastVisible.value) return
    toastVisible.value = false
    toastSwapTimer = setTimeout(() => {
      toastSwapTimer = null
      showNextToast()
    }, TOAST_SWAP_DELAY)
  }

  function showToast(msg: string) {
    toastQueue.push(msg)
    if (!toastVisible.value && !toastSwapTimer) {
      showNextToast()
    }
  }

  function handleToastMouseEnter() {
    toastHovered = true
    clearToastTimer()
  }

  function handleToastMouseLeave() {
    toastHovered = false
    if (toastVisible.value) startToastTimer()
  }

  function invalidateHover() {
    if (toastHovered) handleToastMouseLeave()
  }

  onUnmounted(() => {
    clearToastTimer()
    if (toastSwapTimer) {
      clearTimeout(toastSwapTimer)
      toastSwapTimer = null
    }
  })

  return {
    toastMessage,
    toastVisible,
    toastStyle,
    showToast,
    handleToastMouseEnter,
    handleToastMouseLeave,
    invalidateHover,
  }
}
