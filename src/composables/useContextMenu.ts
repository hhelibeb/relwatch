import { ref, onMounted, onUnmounted } from 'vue'
import { openReleaseUrl } from '../api/client'
import { registerCloser, unregisterCloser, closeAllContextMenus } from './contextMenuBus'

export function useContextMenu() {
  const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)

  function closeContextMenu() {
    contextMenu.value = null
  }

  function handleContextMenu(e: MouseEvent, url: string) {
    closeAllContextMenus()
    contextMenu.value = { x: e.clientX, y: e.clientY, url }
  }

  async function handleCopyLink() {
    try {
      await navigator.clipboard.writeText(contextMenu.value!.url)
    } catch {
      // 静默忽略剪贴板失败
    }
    closeContextMenu()
  }

  function handleOpenLink() {
    const url = contextMenu.value?.url
    if (!url) return
    openReleaseUrl(url)
    closeContextMenu()
  }

  onMounted(() => {
    registerCloser(closeContextMenu)
    document.addEventListener('click', closeContextMenu)
  })
  onUnmounted(() => {
    unregisterCloser(closeContextMenu)
    document.removeEventListener('click', closeContextMenu)
  })

  return {
    contextMenu,
    closeContextMenu,
    handleContextMenu,
    handleCopyLink,
    handleOpenLink,
  }
}
