import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { currentMonitor } from '@tauri-apps/api/window'
import { PhysicalSize } from '@tauri-apps/api/dpi'

// 缩放范围边界：必须与 src-tauri/src/db/settings.rs 的 FONT_SCALE_MIN/MAX 同步修改
const SCALE_MIN = 80
const SCALE_MAX = 150

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
 * 新档位时把窗口物理尺寸同步调整到「100% 基准 × 档位」，保持可见内容量不变：
 * - 基准在非最大化下首次应用时由当前窗口尺寸推导；用户手动 resize 后下次切档
 *   按当前尺寸重推基准（尊重手动调整）；
 * - 最大化时窗口已满屏，跳过尺寸调整（对 maximized 窗口 setSize 有异常还原
 *   行为）且不污染基准——还原后选任意档位仍能从基准正确计算；
 * - 超出当前显示器时钳制到屏幕大小（此时内容区出滚动条，属浏览器缩放的同类
 *   行为），被钳制导致的尺寸偏差不视为手动 resize。
 *
 * 不做按值去重：内建 Ctrl+加减/滚轮手势会绕过本函数修改实际 zoom，「选回当前
 * 档位」必须仍能拉回，去重会造成 UI 无响应。
 *
 * 与 useTheme 同为应用级单例：启动（loadSettings）、设置页选中、保存回填、
 * 放弃修改/保存失败恢复四处都只调 applyFontScale，无局部状态。
 */
// 100% 基准窗口物理尺寸（对应 100 档位）；null = 尚未记录
let baseSize: { width: number; height: number } | null = null
// 上次应用的档位；-1 = 尚未应用（首次按 100 处理：窗口当前尺寸即 100% 基准）
let lastScale = -1
// 上次因最大化跳过了尺寸调整：还原后的窗口尺寸与档位预期不符属预期现象，
// 不得据此重推基准
let sizeSkippedByMaximize = false
// 上次目标尺寸被显示器钳制：实际尺寸与档位预期不符来自物理限制而非手动
// resize，同样不得据此重推基准
let sizeClampedByMonitor = false

export function applyFontScale(percent: number): void {
  const clamped = Math.round(Math.min(SCALE_MAX, Math.max(SCALE_MIN, percent)))
  const prev = lastScale > 0 ? lastScale : 100
  lastScale = clamped
  void applyScale(prev, clamped)
}

async function applyScale(prevScale: number, scale: number): Promise<void> {
  try {
    // 纯浏览器 dev / jsdom 测试环境无 Tauri IPC：getCurrentWebviewWindow()
    // 读取 __TAURI_INTERNALS__ 会同步抛错，invoke 也可能失败——整体静默跳过
    const win = getCurrentWebviewWindow()
    await win.setZoom(scale / 100)
    if (await win.isMaximized()) {
      sizeSkippedByMaximize = true
      return
    }
    const size = await win.innerSize()
    if (baseSize !== null) {
      // 手动 resize 判定：实际尺寸偏离「基准 × 上次档位」的预期值。来自最大化
      // 跳过 / 显示器钳制的偏差不算（见上方标记），否则基准会被污染
      const expectedWidth = (baseSize.width * prevScale) / 100
      const expectedHeight = (baseSize.height * prevScale) / 100
      const drifted =
        Math.abs(size.width - expectedWidth) > 1.5 || Math.abs(size.height - expectedHeight) > 1.5
      if (drifted && !sizeSkippedByMaximize && !sizeClampedByMonitor) {
        baseSize = { width: (size.width * 100) / prevScale, height: (size.height * 100) / prevScale }
      }
    } else {
      baseSize = { width: (size.width * 100) / prevScale, height: (size.height * 100) / prevScale }
    }
    sizeSkippedByMaximize = false
    const monitor = await currentMonitor()
    let width = Math.round((baseSize.width * scale) / 100)
    let height = Math.round((baseSize.height * scale) / 100)
    sizeClampedByMonitor = false
    if (monitor && (width > monitor.size.width || height > monitor.size.height)) {
      width = Math.min(width, monitor.size.width)
      height = Math.min(height, monitor.size.height)
      sizeClampedByMonitor = true
    }
    await win.setSize(new PhysicalSize(width, height))
  } catch {
    // 非 Tauri 环境或 IPC 失败：静默降级
  }
}
