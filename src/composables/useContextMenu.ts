import { ref, onMounted, onUnmounted, inject } from 'vue'
import { openReleaseUrl, copyTextToClipboard } from '../api/client'
import { ShowToastKey } from '../injection-keys'
import { t } from '../i18n'
import { registerCloser, unregisterCloser, closeAllContextMenus } from './contextMenuBus'

export function useContextMenu() {
  const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)
  // 右键菜单复制失败时给用户反馈（未 provide 时静默，component 单测友好）
  const showToast = inject(ShowToastKey, () => {})

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
      showToast(t('release.copy_failed') + (e instanceof Error ? e.message : String(e)))
    }
    closeContextMenu()
  }

  function handleOpenLink() {
    const url = contextMenu.value?.url
    if (!url) return
    openReleaseUrl(url)
    closeContextMenu()
  }

  /** 右键菜单 action 分发：'open' → 打开链接，'copy' → 复制链接 */
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
