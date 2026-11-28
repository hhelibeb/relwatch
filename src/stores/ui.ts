import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { toastDuration } from '../composables/useToast'
import { setErrorToastSink } from '../api/report-error'
import { getPollCountdown } from '../api/releases'
import { formatCountdown } from '../utils'
import {
  overlayRegistryState,
  registerCloser,
  unregisterCloser,
  closeAllContextMenus,
  registerOverlayActive,
  hasActiveOverlay,
  registerFindHandler,
  dispatchFind,
  invalidateTransientUi,
  isTransientUiInvalidated,
  endTransientUiInvalidation,
} from '../composables/contextMenuBus'

export type AppTab = 'sources' | 'releases' | 'logs' | 'settings'

const TOAST_SWAP_DELAY = 240
const TOAST_GAP = 20
const LOGS_REFRESH_DEBOUNCE_MS = 50

export const useUiStore = defineStore('ui', () => {
  const activeTab = ref<AppTab>('sources')
  const logRefreshKey = ref(0)

  const toastMessage = ref('')
  const toastVisible = ref(false)
  const toastQueue = ref<string[]>([])
  let toastTimer: ReturnType<typeof setTimeout> | null = null
  let toastSwapTimer: ReturnType<typeof setTimeout> | null = null
  let toastHovered = false

  const agentPanelOffset = ref({ visible: false, width: 440, dividerWidth: 5 })

  const toastStyle = computed(() => {
    const right = agentPanelOffset.value.visible
      ? agentPanelOffset.value.width + agentPanelOffset.value.dividerWidth + TOAST_GAP
      : TOAST_GAP
    return {
      right: `${right}px`,
      maxWidth: `min(360px, calc(100vw - ${right + TOAST_GAP}px))`,
    }
  })

  function setAgentPanelOffset(visible: boolean, width: number, dividerWidth = 5) {
    agentPanelOffset.value = { visible, width, dividerWidth }
  }

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
    const next = toastQueue.value.shift()
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
    toastQueue.value.push(msg)
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

  function invalidateToastHover() {
    if (toastHovered) handleToastMouseLeave()
  }

  function cleanupToastTimers() {
    clearToastTimer()
    if (toastSwapTimer) {
      clearTimeout(toastSwapTimer)
      toastSwapTimer = null
    }
  }

  setErrorToastSink(showToast)

  // 日志刷新与合帧
  let logsRefreshTimer: ReturnType<typeof setTimeout> | null = null
  function refreshLogs() {
    logRefreshKey.value++
  }

  function scheduleRefreshLogs() {
    if (logsRefreshTimer !== null) return
    logsRefreshTimer = setTimeout(() => {
      logsRefreshTimer = null
      refreshLogs()
    }, LOGS_REFRESH_DEBOUNCE_MS)
  }

  function cleanupLogRefreshTimer() {
    if (logsRefreshTimer !== null) {
      clearTimeout(logsRefreshTimer)
      logsRefreshTimer = null
    }
  }

  // 轮询倒计时
  const countdown = ref('')
  let countdownTimer: ReturnType<typeof setInterval> | null = null
  let countdownSeconds = 0
  let countdownReady = false

  async function syncCountdown(refreshLogsOnJump = true) {
    try {
      const secs = await getPollCountdown()
      const prev = countdownSeconds
      countdownSeconds = secs
      countdown.value = formatCountdown(secs)
      if (refreshLogsOnJump && countdownReady && secs > prev + 30) {
        refreshLogs()
      }
      countdownReady = true
    } catch {
      countdownSeconds = 0
      countdown.value = ''
    }
  }

  function startCountdown() {
    stopCountdown()
    void syncCountdown()
    countdownTimer = setInterval(() => {
      if (countdownSeconds > 0) {
        countdownSeconds--
        countdown.value = formatCountdown(countdownSeconds)
      }
      if (countdownSeconds <= 0) {
        void syncCountdown()
      } else if (countdownSeconds % 60 === 0) {
        void syncCountdown()
      }
    }, 1000)
  }

  function stopCountdown() {
    if (countdownTimer) {
      clearInterval(countdownTimer)
      countdownTimer = null
    }
  }

  // 响应式可观测覆层与瞬态状态
  const hasOverlay = computed(() => {
    void overlayRegistryState.overlayVersion
    return hasActiveOverlay()
  })
  const transientInvalidated = computed(() => overlayRegistryState.transientInvalidated)

  function disposeTimers() {
    cleanupToastTimers()
    cleanupLogRefreshTimer()
    stopCountdown()
  }

  return {
    activeTab,
    logRefreshKey,
    toastMessage,
    toastVisible,
    toastQueue,
    toastStyle,
    setAgentPanelOffset,
    showToast,
    dismissCurrentToast,
    handleToastMouseEnter,
    handleToastMouseLeave,
    invalidateToastHover,
    cleanupToastTimers,
    refreshLogs,
    scheduleRefreshLogs,
    cleanupLogRefreshTimer,
    countdown,
    syncCountdown,
    startCountdown,
    stopCountdown,
    hasOverlay,
    transientInvalidated,
    registerCloser,
    unregisterCloser,
    closeAllContextMenus,
    registerOverlayActive,
    hasActiveOverlay,
    registerFindHandler,
    dispatchFind,
    invalidateTransientUi,
    isTransientUiInvalidated,
    endTransientUiInvalidation,
    disposeTimers,
  }
})
