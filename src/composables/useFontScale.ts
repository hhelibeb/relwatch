import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { currentMonitor } from '@tauri-apps/api/window'
import { PhysicalSize } from '@tauri-apps/api/dpi'

/**
 * 界面缩放单例：把 `font_scale` 百分比（80–150，100 = 默认）应用为 WebView
 * 页面缩放（plugin:webview|set_webview_zoom，等效浏览器 Ctrl+加减）。
 *
 * 选 page zoom 而非 CSS 改造的原因：全项目 font-size 均为硬编码 px，page zoom
 * 在「CSS px 语义不变」的前提下整页等比放大——DOM 弹窗（fixed 定位）、右键菜单、
 * Agent 面板拖拽分隔线的坐标计算均不受影响；独立 WebView（B 站登录窗）不受影响。
 *
 * 页面缩放只改变内容渲染比例、不改窗口物理尺寸，CSS 视口会随缩放变小
 * （150% 下 1200px 窗口仅剩 800 CSS px 宽），元素会挤压/溢出。因此每次应用
 * 新档位时把窗口物理尺寸按新旧比例同步调整，保持可见内容量不变；超出当前
 * 显示器时钳制到屏幕大小（此时内容区出滚动条，属浏览器缩放的同类行为）。
 *
 * 与 useTheme 同为应用级单例：启动（loadSettings）、设置页选中、保存回填、
 * 放弃修改恢复四处都只调 applyFontScale，无局部状态。
 */
let lastApplied = -1

export function applyFontScale(percent: number): void {
  const clamped = Math.round(Math.min(150, Math.max(80, percent)))
  if (clamped === lastApplied) return
  // 首次应用（启动加载持久化档位）时基准视为 100%：窗口当前尺寸即未缩放基准
  const prev = lastApplied > 0 ? lastApplied : 100
  lastApplied = clamped
  void applyScale(prev, clamped)
}

async function applyScale(prevScale: number, scale: number): Promise<void> {
  try {
    // 纯浏览器 dev / jsdom 测试环境无 Tauri IPC：getCurrentWebviewWindow()
    // 读取 __TAURI_INTERNALS__ 会同步抛错，invoke 也可能失败——整体静默跳过
    // （zoom 不生效，窗口也不动）
    const win = getCurrentWebviewWindow()
    await win.setZoom(scale / 100)
    const factor = scale / prevScale
    if (Math.abs(factor - 1) < 1e-9) return
    // 最大化时窗口已满屏、无放大空间，且对 maximized 窗口 setSize 会触发异常
    // 的还原行为——跳过调整，恢复普通状态后由下一次档位切换对齐
    if (await win.isMaximized()) return
    const size = await win.innerSize()
    const monitor = await currentMonitor()
    let width = Math.round(size.width * factor)
    let height = Math.round(size.height * factor)
    if (monitor) {
      width = Math.min(width, monitor.size.width)
      height = Math.min(height, monitor.size.height)
    }
    await win.setSize(new PhysicalSize(width, height))
  } catch {
    // 非 Tauri 环境或 IPC 失败：静默降级
  }
}
