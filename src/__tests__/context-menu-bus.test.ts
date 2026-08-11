import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * contextMenuBus 是模块级单例，每个 import 共享同一份 closers 数组。
 * 用 vi.resetModules + 动态 import 确保每个测试获得干净的实例。
 */
type Bus = typeof import('../composables/contextMenuBus')
let bus: Bus

beforeEach(async () => {
  vi.resetModules()
  bus = await import('../composables/contextMenuBus')
})

// ── registerCloser + closeAllContextMenus ─────────────────────────

describe('registerCloser + closeAllContextMenus', () => {
  it('注册一个 closer，closeAllContextMenus 会调用它', () => {
    const closer = vi.fn()
    bus.registerCloser(closer)
    bus.closeAllContextMenus()
    expect(closer).toHaveBeenCalledOnce()
  })

  it('注册多个 closer，全部被调用', () => {
    const a = vi.fn()
    const b = vi.fn()
    const c = vi.fn()
    bus.registerCloser(a)
    bus.registerCloser(b)
    bus.registerCloser(c)
    bus.closeAllContextMenus()
    expect(a).toHaveBeenCalledOnce()
    expect(b).toHaveBeenCalledOnce()
    expect(c).toHaveBeenCalledOnce()
  })

  it('无 closer 注册时调用不抛错', () => {
    expect(bus.closeAllContextMenus).not.toThrow()
  })
})

// ── unregisterCloser ──────────────────────────────────────────────

describe('unregisterCloser', () => {
  it('注销后 closer 不再被调用', () => {
    const closer = vi.fn()
    bus.registerCloser(closer)
    bus.unregisterCloser(closer)
    bus.closeAllContextMenus()
    expect(closer).not.toHaveBeenCalled()
  })

  it('注销部分 closer，未注销的仍被调用', () => {
    const keep = vi.fn()
    const remove = vi.fn()
    bus.registerCloser(keep)
    bus.registerCloser(remove)
    bus.unregisterCloser(remove)
    bus.closeAllContextMenus()
    expect(keep).toHaveBeenCalledOnce()
    expect(remove).not.toHaveBeenCalled()
  })

  it('注销未注册的函数不报错', () => {
    const closer = vi.fn()
    expect(() => bus.unregisterCloser(closer)).not.toThrow()
  })
})

// ── 迭代安全 ──────────────────────────────────────────────────────

describe('迭代安全', () => {
  it('closer 在执行中注销自身不会导致崩溃', () => {
    const selfUnregister = vi.fn(() => {
      bus.unregisterCloser(selfUnregister)
    })
    const other = vi.fn()
    bus.registerCloser(selfUnregister)
    bus.registerCloser(other)
    expect(() => bus.closeAllContextMenus()).not.toThrow()
    expect(selfUnregister).toHaveBeenCalledOnce()
    expect(other).toHaveBeenCalledOnce()
  })

  it('连续调用 closeAllContextMenus 是安全的', () => {
    const a = vi.fn()
    const b = vi.fn()
    bus.registerCloser(a)
    bus.registerCloser(b)
    bus.closeAllContextMenus()
    bus.closeAllContextMenus()
    expect(a).toHaveBeenCalledTimes(2)
    expect(b).toHaveBeenCalledTimes(2)
  })
})

// ── registerOverlayActive + hasActiveOverlay ─────────────────────

describe('registerOverlayActive + hasActiveOverlay', () => {
  it('无注册时 hasActiveOverlay 返回 false', () => {
    expect(bus.hasActiveOverlay()).toBe(false)
  })

  it('注册活跃回调后返回 true，注销后返回 false', () => {
    const unregister = bus.registerOverlayActive(() => true)
    expect(bus.hasActiveOverlay()).toBe(true)
    unregister()
    expect(bus.hasActiveOverlay()).toBe(false)
  })

  it('任一回调返回 true 即为有覆盖层', () => {
    const a = bus.registerOverlayActive(() => false)
    const b = bus.registerOverlayActive(() => true)
    expect(bus.hasActiveOverlay()).toBe(true)
    a()
    b()
    expect(bus.hasActiveOverlay()).toBe(false)
  })

  it('回调在判定过程中注销自身不会导致崩溃或跳项', () => {
    const unregister = bus.registerOverlayActive(() => {
      unregister()
      return true
    })
    expect(() => bus.hasActiveOverlay()).not.toThrow()
    expect(bus.hasActiveOverlay()).toBe(false)
  })

  it('重复注销同一个回调是安全的', () => {
    const unregister = bus.registerOverlayActive(() => true)
    unregister()
    expect(() => unregister()).not.toThrow()
  })
})
