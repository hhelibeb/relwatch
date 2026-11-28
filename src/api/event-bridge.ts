import type { UnlistenFn } from '@tauri-apps/api/event'
import { events, type ReleaseTranslateChunk } from '../bindings'
import { t } from '../i18n'
import { useUiStore, type AppTab } from '../stores/ui'
import { useCatalogStore } from '../stores/catalog'
import { useReleaseFilterStore } from '../stores/releaseFilter'
import { useAgentStore } from '../stores/agent'

type TranslateChunkHandler = (payload: ReleaseTranslateChunk) => void

const translateChunkSubscribers = new Set<TranslateChunkHandler>()
let bridgeEpoch = 0
let activeBridgeUnmount: (() => void) | null = null

export function isEventBridgeMounted(): boolean {
  return activeBridgeUnmount !== null
}

export async function subscribeTranslateChunk(handler: TranslateChunkHandler): Promise<() => void> {
  if (isEventBridgeMounted()) {
    translateChunkSubscribers.add(handler)
    return () => {
      translateChunkSubscribers.delete(handler)
    }
  }

  let active = true
  const off = await events.releaseTranslateChunk.listen((e) => {
    if (active) handler(e.payload)
  })
  return () => {
    if (!active) return
    active = false
    off()
  }
}

export async function mountEventBridge(): Promise<() => void> {
  if (activeBridgeUnmount) {
    activeBridgeUnmount()
  }

  const ui = useUiStore()
  const catalog = useCatalogStore()
  const releaseFilter = useReleaseFilterStore()
  const agent = useAgentStore()

  const unlisteners: UnlistenFn[] = []
  const myEpoch = ++bridgeEpoch

  const unmount = () => {
    if (activeBridgeUnmount === unmount) {
      activeBridgeUnmount = null
    }
    if (bridgeEpoch === myEpoch) {
      bridgeEpoch++
    }
    translateChunkSubscribers.clear()
    for (const off of unlisteners) {
      off()
    }
    unlisteners.length = 0
  }
  activeBridgeUnmount = unmount

  const register = async (promise: Promise<UnlistenFn>) => {
    const off = await promise
    if (bridgeEpoch !== myEpoch) {
      off()
      return
    }
    unlisteners.push(off)
  }

  try {
    await register(
      events.navigate.listen((event) => {
        if (
          event.payload === 'sources'
          || event.payload === 'releases'
          || event.payload === 'logs'
          || event.payload === 'settings'
        ) {
          ui.activeTab = event.payload as AppTab
        }
      }),
    )

    await register(
      events.focusRelease.listen((event) => {
        releaseFilter.focusReleaseById(event.payload)
      }),
    )

    await register(
      events.pollCompleted.listen(() => {
        void catalog.loadSources()
        void catalog.loadReleases()
        ui.refreshLogs()
        void ui.syncCountdown(false)
      }),
    )

    await register(
      events.releaseStateChanged.listen(() => {
        catalog.scheduleLoadReleases()
        ui.scheduleRefreshLogs()
      }),
    )

    await register(
      events.logAppended.listen(() => {
        ui.scheduleRefreshLogs()
      }),
    )

    await register(
      events.sourceAutoDisabled.listen((event) => {
        ui.showToast(
          t(
            'app.source_auto_disabled',
            event.payload.owner,
            event.payload.repo,
            String(event.payload.failures),
          ),
        )
        void catalog.loadSources()
        ui.refreshLogs()
      }),
    )

    await register(
      events.agentRunFinished.listen((event) => {
        void agent.onRunFinished(event.payload.session_key)
      }),
    )

    await register(
      events.agentRpcStream.listen((event) => {
        agent.handleRpcStream(event.payload)
      }),
    )

    await register(
      events.releaseTranslateChunk.listen((event) => {
        for (const fn of [...translateChunkSubscribers]) {
          fn(event.payload)
        }
      }),
    )
  } catch (err) {
    unmount()
    throw err
  }

  return unmount
}
