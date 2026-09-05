import { beforeEach, describe, expect, it, vi } from 'vitest'

// useFontScale 的内部状态（baseSize/lastScale/标记）是模块级的，mock 全部走
// vi.hoisted 共享引用 + 每用例 vi.resetModules 重新加载，保证用例间互不污染
const mocks = vi.hoisted(() => ({
  setZoom: vi.fn(),
  innerSize: vi.fn(),
  isMaximized: vi.fn(),
  setSize: vi.fn(),
  monitor: { size: { width: 2560, height: 1440 } },
  throwOnGetCurrent: false,
}))

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => {
    // 与真实行为一致：无 __TAURI_INTERNALS__ 的环境同步抛错
    if (mocks.throwOnGetCurrent) throw new TypeError('Cannot read properties of undefined')
    return { setZoom: mocks.setZoom, innerSize: mocks.innerSize, isMaximized: mocks.isMaximized, setSize: mocks.setSize }
  },
}))
vi.mock('@tauri-apps/api/window', () => ({
  currentMonitor: () => Promise.resolve(mocks.monitor),
}))
vi.mock('@tauri-apps/api/dpi', () => {
  class PhysicalSize {
    constructor(
      public width: number,
      public height: number,
    ) {}
  }
  return { PhysicalSize }
})

async function loadModule() {
  vi.resetModules()
  return await import('../composables/useFontScale')
}

/** applyFontScale 内部是 fire-and-forget 异步链，flush 一轮宏任务等它跑完 */
const flush = () => new Promise(resolve => setTimeout(resolve, 0))

function setWindow(width: number, height: number) {
  mocks.innerSize.mockResolvedValue({ width, height })
}

function setSizeCalls(): Array<[number, number]> {
  return mocks.setSize.mock.calls.map(call => [call[0].width, call[0].height] as [number, number])
}

beforeEach(() => {
  vi.clearAllMocks()
  mocks.setZoom.mockResolvedValue(undefined)
  mocks.isMaximized.mockResolvedValue(false)
  mocks.setSize.mockResolvedValue(undefined)
  setWindow(1200, 800)
  mocks.monitor = { size: { width: 2560, height: 1440 } }
  mocks.throwOnGetCurrent = false
})

describe('applyFontScale', () => {
  it('clamp 到 [80, 150] 边界', async () => {
    const mod = await loadModule()
    mod.applyFontScale(0)
    await flush()
    expect(mocks.setZoom).toHaveBeenLastCalledWith(0.8)

    vi.clearAllMocks()
    mod.applyFontScale(999)
    await flush()
    expect(mocks.setZoom).toHaveBeenLastCalledWith(1.5)
  })

  it('启动应用 100%：以当前窗口尺寸记基准，setSize 为 no-op 等值', async () => {
    const mod = await loadModule()
    mod.applyFontScale(100)
    await flush()
    expect(mocks.setZoom).toHaveBeenCalledWith(1)
    expect(setSizeCalls()).toEqual([[1200, 800]])
  })

  it('切档窗口按「基准 × 档位」等比调整', async () => {
    const mod = await loadModule()
    mod.applyFontScale(100)
    await flush()
    mod.applyFontScale(150)
    await flush()
    expect(setSizeCalls()).toEqual([[1200, 800], [1800, 1200]])
  })

  it('不做按值去重：选回当前档位仍执行 setZoom（手势可能已改实际 zoom）', async () => {
    const mod = await loadModule()
    mod.applyFontScale(125)
    await flush()
    mod.applyFontScale(125)
    await flush()
    expect(mocks.setZoom).toHaveBeenCalledTimes(2)
    expect(mocks.setZoom).toHaveBeenLastCalledWith(1.25)
  })

  it('最大化跳过 setSize 且不污染基准：还原后选回旧档窗口尺寸正确', async () => {
    const mod = await loadModule()
    mod.applyFontScale(100)
    await flush()
    mocks.isMaximized.mockResolvedValue(true)
    mod.applyFontScale(150)
    await flush()
    expect(mocks.setZoom).toHaveBeenLastCalledWith(1.5)
    expect(mocks.setSize).toHaveBeenCalledTimes(1) // 最大化未 setSize

    // 还原窗口（尺寸仍是 100 档的 1200x800，显示着 150% 内容），选回 100%：
    // 链式比例会算出 800x533，基准模型应还原为 1200x800
    mocks.isMaximized.mockResolvedValue(false)
    setWindow(1200, 800)
    mod.applyFontScale(100)
    await flush()
    expect(setSizeCalls().at(-1)).toEqual([1200, 800])
  })

  it('手动 resize 后切档：按当前尺寸重推基准，尊重手动调整', async () => {
    const mod = await loadModule()
    mocks.monitor = { size: { width: 3840, height: 2160 } } // 本用例不涉及 clamp
    mod.applyFontScale(100)
    await flush()
    setWindow(1600, 1100) // 用户手动拖大（150% 档位下）
    mod.applyFontScale(150)
    await flush()
    expect(setSizeCalls().at(-1)).toEqual([2400, 1650])
  })

  it('显示器钳制后的尺寸偏差不误判为手动 resize', async () => {
    const mod = await loadModule()
    mod.applyFontScale(100)
    await flush()
    mocks.monitor = { size: { width: 1600, height: 900 } }
    mod.applyFontScale(150)
    await flush()
    expect(setSizeCalls().at(-1)).toEqual([1600, 900]) // 被 clamp

    // 切回中间档：不应把 clamp 后尺寸当作 150% 真实尺寸重推基准
    setWindow(1600, 900)
    mod.applyFontScale(110)
    await flush()
    expect(setSizeCalls().at(-1)).toEqual([1320, 880]) // 1200x800 基准 × 1.1
  })

  it('目标超屏时钳制到显示器大小', async () => {
    const mod = await loadModule()
    mocks.monitor = { size: { width: 1600, height: 1100 } }
    mod.applyFontScale(150) // 基准 1200x800 × 1.5 = 1800x1200 超屏
    await flush()
    expect(setSizeCalls().at(-1)).toEqual([1600, 1100])
  })

  it('非 Tauri 环境（getCurrentWebviewWindow 同步抛错）静默降级不抛错', async () => {
    const mod = await loadModule()
    mocks.throwOnGetCurrent = true
    expect(() => mod.applyFontScale(125)).not.toThrow()
    await flush()
  })
})
