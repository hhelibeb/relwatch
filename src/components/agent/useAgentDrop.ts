import { ref, type Ref } from 'vue'
import type { AgentEntityRefSeed } from '../../injection-keys'
import { isEntityDrag, parseEntityDrag } from './agentChatUtils'

export interface UseAgentDropOptions {
  findScopeInside: Ref<boolean>
  onAttachEntity: (entity: AgentEntityRefSeed) => void
  onStartNewSession: () => void
}

export function useAgentDrop(options: UseAgentDropOptions) {
  const dragOver = ref(false)
  const headerDropOver = ref(false)

  function onDragOver(e: DragEvent) {
    if (!isEntityDrag(e)) return
    e.preventDefault()
    dragOver.value = true
    options.findScopeInside.value = true
  }

  function onDragLeave() {
    dragOver.value = false
  }

  function handleDrop(e: DragEvent) {
    if (!isEntityDrag(e)) return
    e.preventDefault()
    dragOver.value = false
    const entity = parseEntityDrag(e)
    if (entity) options.onAttachEntity(entity)
  }

  function onHeaderDragOver(e: DragEvent) {
    if (!isEntityDrag(e)) return
    e.preventDefault()
    headerDropOver.value = true
    options.findScopeInside.value = true
  }

  function onHeaderDragLeave(e: DragEvent) {
    const el = e.currentTarget as HTMLElement
    if (!el.contains(e.relatedTarget as Node | null)) {
      headerDropOver.value = false
    }
  }

  function handleDropNewSession(e: DragEvent) {
    if (!isEntityDrag(e)) return
    e.preventDefault()
    e.stopPropagation()
    dragOver.value = false
    headerDropOver.value = false
    const entity = parseEntityDrag(e)
    if (entity) {
      options.onStartNewSession()
      options.onAttachEntity(entity)
    }
  }

  return {
    dragOver,
    headerDropOver,
    onDragOver,
    onDragLeave,
    handleDrop,
    onHeaderDragOver,
    onHeaderDragLeave,
    handleDropNewSession,
  }
}
