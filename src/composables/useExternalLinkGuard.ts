import { onMounted, onUnmounted } from 'vue'
import { openReleaseUrl } from '../api/client'
import { closeAllContextMenus } from './contextMenuBus'

// 全局链接守卫：捕获阶段拦截一切 <a> 点击——http(s) 绝对链接交给系统浏览器，
// 其他（相对路径/锚点等）直接吞掉。webview 自身导航一旦发生用户无法回到应用
// （无后退机制），任何情况下都不允许。
export function useExternalLinkGuard() {
  function handleClick(e: MouseEvent) {
    if (e.button !== 0 || e.defaultPrevented) return
    const anchor = (e.target as HTMLElement | null)?.closest?.('a[href]') as HTMLAnchorElement | null
    if (!anchor) return
    e.preventDefault()
    // 阻止事件继续传播（如 ReleaseItem 预览的「点击打开详情」不应再触发）
    e.stopPropagation()
    // 注意用 getAttribute 取原始 href：a.href 会把相对路径绝对化为 webview 源。
    // markdown 里的相对链接指向源站路径，无法可靠推断绝对地址——同样吞掉，
    // 既不导航 webview 也不在浏览器打开无效 URL。
    const rawHref = anchor.getAttribute('href') ?? ''
    if (!/^https?:\/\//i.test(rawHref)) return
    closeAllContextMenus()
    openReleaseUrl(anchor.href)
  }

  onMounted(() => document.addEventListener('click', handleClick, true))
  onUnmounted(() => document.removeEventListener('click', handleClick, true))
}
