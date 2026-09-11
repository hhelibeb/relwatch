import { onMounted, onUnmounted, type Ref } from 'vue'
import { commands } from '../bindings'
import { hasActiveOverlay } from './contextMenuBus'

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

/** 会因 Esc 而失焦、且"退出输入框"语义成立的输入元素类型 */
const TEXT_INPUT_TYPES = new Set([
  'text',
  'search',
  'number',
  'password',
  'url',
  'email',
  'tel',
  '', // 未显式声明 type 的 <input> 默认即为 text
])

/**
 * 该元素是否属于「Esc 应先让它失焦」的输入元素。
 * 排除 checkbox/radio/button 等——勾选框不承载"正在输入"语义，
 * 对它按 Esc 应当是直接退出应用，而不是给一个几乎无感的失焦。
 */
function isTextField(el: Element | null): el is HTMLElement {
  if (!el) return false
  const tag = el.tagName
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true
  if (tag === 'INPUT') {
    return TEXT_INPUT_TYPES.has((el as HTMLInputElement).type.toLowerCase())
  }
  const html = el as HTMLElement
  // 不用 isContentEditable：jsdom 未实现（恒为 undefined）会让该分支在测试中失效。
  // contentEditable 属性与 contenteditable 特性在浏览器中均可靠。
  const attr = html.getAttribute('contenteditable')
  return attr === '' || attr === 'true' || attr === 'plaintext-only' || html.contentEditable === 'true'
}

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
