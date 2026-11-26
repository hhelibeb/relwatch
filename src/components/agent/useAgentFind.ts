import { computed, inject, onMounted, onUnmounted, ref, watch, type Ref } from 'vue'
import type { AgentChatMessage } from '../../api/agent'
import type { Source } from '../../api/sources'
import type { ReleaseInfo } from '../../api/releases'
import { ActiveTabKey } from '../../injection-keys'
import { registerOverlayActive } from '../../composables/contextMenuBus'
import { useElementFind } from '../../composables/useElementFind'

const PANEL_OWNED_SELECTOR = '.agent-ws-menu, .agent-divider'

export function blocksFingerprint(blocks: AgentChatMessage['blocks']): string {
  let out = ''
  for (const b of blocks) {
    switch (b.kind) {
      case 'text':
      case 'thinking':
      case 'toolResult':
        out += `${b.kind}:${b.text.length}:${b.text.slice(0, 12)}:${b.text.slice(-12)};`
        break
      case 'toolCall':
        out += `toolCall:${b.name}:${b.args.length}:${b.args.slice(0, 12)};`
        break
      case 'bash':
        out += `bash:${b.command.length}:${b.command.slice(0, 12)}:${b.output.length}:${b.output.slice(-12)};`
        break
      default: {
        const unhandled: never = b
        out += `${String(unhandled)};`
      }
    }
  }
  return out
}

export interface AgentFindDecorationLike {
  main: string
  folded?: string | null
  entities: { kind: string; id: number }[]
  run?: { status?: string; error?: string | null } | null
}

export interface UseAgentFindOptions {
  rootEl: Ref<HTMLElement | null>
  scrollRef: Ref<HTMLElement | null>
  activeKey: Ref<string>
  displayedMessages: Ref<readonly AgentChatMessage[]>
  messageDecorations: Ref<readonly (AgentFindDecorationLike | null | undefined)[]>
  sources: Ref<readonly Source[]>
  releases: Ref<readonly ReleaseInfo[]>
  isStreaming: () => boolean
}

export function useAgentFind(options: UseAgentFindOptions) {
  const findScopeInside = ref(false)
  const findActive = ref(false)
  let unregisterFindOverlay: (() => void) | null = null

  function trackFindScope(e: Event) {
    const root = options.rootEl.value
    const target = e.target as Node | null
    if (!root || !target) {
      findScopeInside.value = false
      return
    }
    findScopeInside.value =
      root.contains(target)
      || (target instanceof Element && target.closest(PANEL_OWNED_SELECTOR) !== null)
  }

  const injectedActiveTab = inject(ActiveTabKey, null)
  if (injectedActiveTab) {
    watch(injectedActiveTab, () => {
      findScopeInside.value = false
    })
  }

  function decorationsFingerprint(): string {
    let out = ''
    for (const d of options.messageDecorations.value) {
      if (!d) {
        out += '-;'
        continue
      }
      out += `${d.main.length}:${d.folded?.length ?? 0}:${d.entities.map((e) => `${e.kind}${e.id}`).join('.')}:${d.run?.status ?? ''}:${d.run?.error ?? ''};`
    }
    out += `${options.sources.value.length}/${options.releases.value.length}`
    return out
  }

  const findContentSource = computed(() => {
    if (!findActive.value) return ''
    return `${options.activeKey.value}|`
      + options.displayedMessages.value
        .map((m) => `${m.role}@${m.timestamp}#${blocksFingerprint(m.blocks)}`)
        .join('|')
      + `|${decorationsFingerprint()}`
  })

  const {
    open: findOpen,
    query: findQuery,
    inputEl: findInputEl,
    matchCount: findMatchCount,
    currentIndex: findCurrentIndex,
    openFind,
    closeFind,
    next: nextFindMatch,
    prev: prevFindMatch,
    handleInputKeydown: handleFindInputKeydown,
  } = useElementFind({
    container: options.scrollRef,
    skipSelector: 'button, .md-code-lang',
    contentSource: () => findContentSource.value,
    isStreaming: options.isStreaming,
    shouldHandle: () => findScopeInside.value,
  })

  function setFindInputEl(el: HTMLInputElement | null) {
    findInputEl.value = el
  }

  function toggleFind() {
    if (findOpen.value) closeFind()
    else openFind()
  }

  function handleFindKeydown(e: KeyboardEvent) {
    if (!findOpen.value || e.isComposing) return
    if ((e.target as HTMLElement | null)?.hasAttribute?.('data-esc-local')) return
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      closeFind()
    } else if (e.key === 'F3') {
      e.preventDefault()
      if (e.shiftKey) prevFindMatch()
      else nextFindMatch()
    }
  }

  watch(findOpen, (open) => {
    findActive.value = open
    if (open) {
      unregisterFindOverlay = registerOverlayActive(() => findOpen.value)
      document.addEventListener('keydown', handleFindKeydown, true)
    } else {
      unregisterFindOverlay?.()
      unregisterFindOverlay = null
      document.removeEventListener('keydown', handleFindKeydown, true)
    }
  })

  onMounted(() => {
    document.addEventListener('pointerdown', trackFindScope, true)
    document.addEventListener('focusin', trackFindScope, true)
    document.addEventListener('wheel', trackFindScope, { capture: true, passive: true })
  })

  onUnmounted(() => {
    document.removeEventListener('pointerdown', trackFindScope, true)
    document.removeEventListener('focusin', trackFindScope, true)
    document.removeEventListener('wheel', trackFindScope, { capture: true })
    document.removeEventListener('keydown', handleFindKeydown, true)
    unregisterFindOverlay?.()
    unregisterFindOverlay = null
  })

  return {
    findScopeInside,
    findOpen,
    findQuery,
    findMatchCount,
    findCurrentIndex,
    setFindInputEl,
    toggleFind,
    closeFind,
    nextFindMatch,
    prevFindMatch,
    handleFindInputKeydown,
  }
}
