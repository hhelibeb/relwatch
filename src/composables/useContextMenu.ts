import { ref, onMounted, onUnmounted } from 'vue'
import { openReleaseUrl, copyTextToClipboard } from '../api/client'
import { extractErrorMessage } from '../services/feedback'
import { useUiStore } from '../stores/ui'
import { t } from '../i18n'
import { registerCloser, unregisterCloser, closeAllContextMenus } from './contextMenuBus'

export function useContextMenu() {
  const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)
  const ui = useUiStore()

  function closeContextMenu() {
    contextMenu.value = null
  }

  function handleContextMenu(e: MouseEvent, url: string) {
    closeAllContextMenus()
    contextMenu.value = { x: e.clientX, y: e.clientY, url }
  }

  async function handleCopyLink() {
    try {
      await copyTextToClipboard(contextMenu.value!.url)
    } catch (e: unknown) {
      ui.showToast(t('release.copy_failed') + extractErrorMessage(e))
    }
    closeContextMenu()
  }

  function handleOpenLink() {
    const url = contextMenu.value?.url
    if (!url) return
    openReleaseUrl(url)
    closeContextMenu()
  }

  function handleMenuAction(id: string) {
    if (id === 'open') handleOpenLink()
    else if (id === 'copy') handleCopyLink()
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
    handleMenuAction,
  }
}
