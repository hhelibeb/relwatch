import { onMounted, onUnmounted, type Ref } from 'vue'
import { commands } from '../bindings'
import { hasActiveOverlay } from './contextMenuBus'
import { isTextField } from '../utils/dom'

/**
 * Escape 逐层退出——最外层最小化到托盘
 *
 * 层级（自上而下，先命中者处理，本轮结束）：
 * 1. IME 组合期 → 完全不介入（交给输入法处理候选词，例如取消上屏）
 * 2. 有覆盖层打开 → 让子组件处理（右键菜单/下拉/弹窗/面板）
 * 3. 焦点在文本输入元素 → 退出输入框（blur），不退出应用
 * 4. 以上都不满足 + 已开启最小化到托盘 → 隐藏窗口到系统托盘
 *
 * 「第一次 Esc 退出输入框、第二次 Esc 退出应用」由第 3 步自然实现：
 * blur() 同步生效（同一次事件内 activeElement 已是 body），因此第二次按下时
 * 第 3 步不再命中，直接落到第 4 步——不需要任何跨事件的"记忆"状态。
 *
 * 覆盖层判定走 contextMenuBus 的注册表（registerOverlayActive）：
 * 所有覆盖层（右键菜单/下拉/弹窗/面板）打开期间自行注册活跃回调，
 * 新增覆盖层时无需再改本文件——漏注册才会导致 Esc 误触最小化到托盘。
 */

export function useEscapeToTray(minimizeToTray: Ref<boolean>) {
  function handleKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return

    // 1. 输入法组合期：Esc 属于输入法（取消候选词），不 blur 不退出
    if (e.isComposing) return

    // 2. 有覆盖层打开 → 让子组件的 Escape 处理器优先处理
    if (hasActiveOverlay()) return

    // 3. 焦点在文本输入元素 → 第一次 Esc 只退出输入框，不退出应用。
    //    data-esc-local 标记的控件自行处理 Esc（如会话重命名 = 取消重命名），
    //    全局必须整体不介入：既不能抢在它前面 blur（会先触发其 @blur 副作用），
    //    也不能落到第 4 步（那就是"输入框里的 Esc 直接退出应用"，同样不对）。
    const active = document.activeElement
    if (isTextField(active)) {
      if (active.hasAttribute('data-esc-local')) return
      e.preventDefault()
      active.blur()
      return
    }

    // 4. 用户未开启最小化到托盘
    if (!minimizeToTray.value) return

    e.preventDefault()
    commands.hideToTray()
  }

  onMounted(() => document.addEventListener('keydown', handleKeydown, true))
  onUnmounted(() => document.removeEventListener('keydown', handleKeydown, true))
}
