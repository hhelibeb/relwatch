import { computed, onMounted, onUnmounted, ref, watch, type Ref } from 'vue'
import { currentMonitor, getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
import { commands } from '../bindings'
import type { AgentWorkspaceSeed } from '../types/agent'
import { extractErrorMessage } from '../services/feedback'
import { track } from './useUsageTracking'

export const AGENT_PANEL_DEFAULT_WIDTH = 440
export const AGENT_PANEL_MIN_WIDTH = 280
export const MAIN_MIN_WIDTH = 710
export const MAIN_MIN_HEIGHT = 450
export const AGENT_DIVIDER_WIDTH = 5

export function clampMainWidth(w: number, totalW: number): number {
  const maxW = Math.max(MAIN_MIN_WIDTH, totalW - AGENT_DIVIDER_WIDTH - AGENT_PANEL_MIN_WIDTH)
  return Math.round(Math.min(Math.max(w, MAIN_MIN_WIDTH), maxW))
}

export function agentWindowMinWidth(mainW: number): number {
  return mainW + AGENT_DIVIDER_WIDTH + AGENT_PANEL_MIN_WIDTH
}

export interface UseAgentPanelLayoutOptions {
  agentEnabled: Ref<boolean>
  onError: (msg: string) => void
}

export function useAgentPanelLayout(options: UseAgentPanelLayoutOptions) {
  const agentPanelOpen = ref(false)
  const agentPanelSeed = ref<AgentWorkspaceSeed | null>(null)
  const mainWidth = ref(window.innerWidth)
  const viewportWidth = ref(window.innerWidth)

  const agentPanelWidth = computed(() =>
    Math.max(AGENT_PANEL_MIN_WIDTH, viewportWidth.value - mainWidth.value - AGENT_DIVIDER_WIDTH),
  )
  const agentPanelVisible = computed(() => agentPanelOpen.value && options.agentEnabled.value)

  let panelBusy = false
  let panelWidthSaveTimer: ReturnType<typeof setTimeout> | null = null
  let lastMinWidth = -1

  function cancelPanelWidthSave() {
    if (panelWidthSaveTimer) {
      clearTimeout(panelWidthSaveTimer)
      panelWidthSaveTimer = null
    }
  }

  function persistPanelWidth(w: number) {
    void commands.saveAgentWsWidth(w).catch(() => {})
  }

  function schedulePanelWidthSave() {
    cancelPanelWidthSave()
    const w = agentPanelWidth.value
    panelWidthSaveTimer = setTimeout(() => {
      panelWidthSaveTimer = null
      persistPanelWidth(w)
    }, 600)
  }

  function onViewportResize() {
    const w = window.innerWidth
    // 窗口最小化/隐藏到托盘时 WebView 会报 0：直接写回会把面板宽度算成最小值并
    // 防抖落库（下次打开永远 280px）；面板关闭态下还会把 mainWidth 写成 0，
    // 恢复后反向算出超宽面板。尺寸不可信时直接忽略这次 resize。
    if (w <= 0) return
    viewportWidth.value = w
    if (panelBusy) return
    if (!agentPanelOpen.value) {
      mainWidth.value = viewportWidth.value
      return
    }
    schedulePanelWidthSave()
  }

  let dividerDragging = false
  let dividerStartX = 0
  let dividerStartMainWidth = 0

  function endDividerDrag() {
    if (!dividerDragging) return
    dividerDragging = false
    document.removeEventListener('mousemove', onDividerMouseMove)
    document.removeEventListener('mouseup', onDividerMouseUp)
    window.removeEventListener('blur', endDividerDrag)
    document.body.classList.remove('agent-resizing')
    schedulePanelWidthSave()
    syncWindowMinWidth()
  }

  function onDividerMouseMove(e: MouseEvent) {
    if (!dividerDragging) return
    // 指针在窗口外松开 / 被其它应用抢走输入时收不到 mouseup，靠按键已抬起兜底，
    // 否则 col-resize 光标与拖拽态会永久卡死
    if ((e.buttons & 1) === 0) {
      endDividerDrag()
      return
    }
    mainWidth.value = clampMainWidth(
      dividerStartMainWidth + (e.clientX - dividerStartX),
      viewportWidth.value,
    )
  }

  function onDividerMouseUp() {
    endDividerDrag()
  }

  function onDividerMouseDown(e: MouseEvent) {
    if (e.button !== 0) return
    e.preventDefault()
    dividerDragging = true
    dividerStartX = e.clientX
    dividerStartMainWidth = clampMainWidth(mainWidth.value, viewportWidth.value)
    mainWidth.value = dividerStartMainWidth
    document.addEventListener('mousemove', onDividerMouseMove)
    document.addEventListener('mouseup', onDividerMouseUp)
    window.addEventListener('blur', endDividerDrag)
    document.body.classList.add('agent-resizing')
  }

  async function loadSavedPanelWidth(): Promise<number> {
    try {
      const saved = await commands.getAgentWsWidth()
      return saved > 0 ? saved : AGENT_PANEL_DEFAULT_WIDTH
    } catch {
      return AGENT_PANEL_DEFAULT_WIDTH
    }
  }

  async function setWindowMinWidth(win: ReturnType<typeof getCurrentWindow>, width: number) {
    if (width === lastMinWidth) return
    lastMinWidth = width
    try {
      await win.setMinSize(new LogicalSize(width, MAIN_MIN_HEIGHT))
    } catch {
      lastMinWidth = -1
    }
  }

  function syncWindowMinWidth() {
    const w = agentPanelVisible.value
      ? agentWindowMinWidth(mainWidth.value)
      : MAIN_MIN_WIDTH
    void setWindowMinWidth(getCurrentWindow(), w)
  }

  async function rightFreeSpace(win: ReturnType<typeof getCurrentWindow>): Promise<number> {
    try {
      const monitor = await currentMonitor()
      if (!monitor) return Number.POSITIVE_INFINITY
      const scale = monitor.scaleFactor || 1
      const pos = await win.outerPosition()
      const size = await win.outerSize()
      const monitorRight = (monitor.position.x + monitor.size.width) / scale
      const windowRight = (pos.x + size.width) / scale
      return Math.max(0, monitorRight - windowRight)
    } catch {
      return Number.POSITIVE_INFINITY
    }
  }

  async function openAgentWorkspace(seed?: AgentWorkspaceSeed) {
    if (!options.agentEnabled.value) return
    agentPanelSeed.value = seed ?? null
    if (agentPanelOpen.value || panelBusy) return
    panelBusy = true
    try {
      const panelTarget = Math.max(await loadSavedPanelWidth(), AGENT_PANEL_MIN_WIDTH)
      const win = getCurrentWindow()
      const size = await win.innerSize()
      const scale = await win.scaleFactor()
      const windowW = size.width / scale
      const height = size.height / scale
      const layoutW = window.innerWidth
      const pxPerLogical = windowW > 0 ? layoutW / windowW : 1
      let grow = 0
      if (!(await win.isMaximized())) {
        grow = Math.max(
          0,
          Math.min(panelTarget + AGENT_DIVIDER_WIDTH, await rightFreeSpace(win)),
        )
        if (grow > 0) await win.setSize(new LogicalSize(windowW + grow, height))
      }
      const availableW = layoutW + grow * pxPerLogical
      mainWidth.value = clampMainWidth(availableW - panelTarget - AGENT_DIVIDER_WIDTH, availableW)
      viewportWidth.value = availableW
      await setWindowMinWidth(win, agentWindowMinWidth(mainWidth.value))
      agentPanelOpen.value = true
    } catch (e) {
      options.onError(extractErrorMessage(e))
    } finally {
      panelBusy = false
    }
  }

  async function closeAgentPanel() {
    if (panelBusy) return
    panelBusy = true
    cancelPanelWidthSave()
    persistPanelWidth(agentPanelWidth.value)
    agentPanelOpen.value = false
    try {
      const win = getCurrentWindow()
      await setWindowMinWidth(win, MAIN_MIN_WIDTH)
      if (await win.isMaximized()) return
      const scale = await win.scaleFactor()
      const size = await win.innerSize()
      const targetW = Math.max(mainWidth.value, MAIN_MIN_WIDTH)
      await win.setSize(new LogicalSize(targetW, size.height / scale))
    } catch {
      // 恢复尺寸失败不影响面板关闭
    } finally {
      panelBusy = false
    }
  }

  async function toggleAgentWorkspace() {
    if (agentPanelOpen.value) {
      track('release.collapse_agent_workspace')
      await closeAgentPanel()
    } else {
      track('release.open_agent_workspace')
      await openAgentWorkspace()
    }
  }

  watch(options.agentEnabled, (enabled) => {
    if (!enabled && agentPanelOpen.value) {
      void closeAgentPanel()
    }
  })

  watch(agentPanelVisible, (visible) => {
    syncWindowMinWidth()
    if (visible) return
    if (agentPanelOpen.value) void closeAgentPanel()
  }, { immediate: true })

  onMounted(() => {
    window.addEventListener('resize', onViewportResize)
  })

  onUnmounted(() => {
    window.removeEventListener('resize', onViewportResize)
    cancelPanelWidthSave()
    if (dividerDragging) {
      dividerDragging = false
      document.removeEventListener('mousemove', onDividerMouseMove)
      document.removeEventListener('mouseup', onDividerMouseUp)
      window.removeEventListener('blur', endDividerDrag)
      document.body.classList.remove('agent-resizing')
    }
  })

  return {
    agentPanelOpen,
    agentPanelSeed,
    agentPanelWidth,
    agentPanelVisible,
    mainWidth,
    onDividerMouseDown,
    openAgentWorkspace,
    closeAgentPanel,
    toggleAgentWorkspace,
  }
}
