import { describe, it, expect, vi, afterEach } from 'vitest'
import { defineComponent, ref, onMounted, onUnmounted } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'

// Mock Tauri event API
const mockUnlisten = vi.fn()
const mockListen = vi.fn((_event: string, _handler: Function) =>
  Promise.resolve(mockUnlisten)
)

vi.mock('@tauri-apps/api/event', () => ({
  listen: mockListen,
}))

afterEach(() => {
  vi.clearAllMocks()
})

/**
 * 模拟 App.vue 的 Tauri 事件监听模式：
 * 在 onMounted 中注册监听器并存储 unlisten 函数，
 * 在 onUnmounted 中调用所有 unlisten 函数进行清理。
 */
const EventApp = defineComponent({
  emits: ['events-registered'],
  setup(_props, { emit }) {
    const unlisteners: (() => void)[] = []

    onMounted(async () => {
      const navUnlisten = await mockListen('navigate', () => {})
      unlisteners.push(navUnlisten)

      const pollUnlisten = await mockListen('poll-completed', () => {})
      unlisteners.push(pollUnlisten)

      const stateUnlisten = await mockListen('release-state-changed', () => {})
      unlisteners.push(stateUnlisten)

      emit('events-registered', unlisteners.length)
    })

    onUnmounted(() => {
      for (const unlisten of unlisteners) {
        unlisten()
      }
      unlisteners.length = 0
    })

    return {}
  },
  template: '<div />',
})

describe('App.vue — 事件监听器注册与清理', () => {
  it('在 onMounted 中注册 3 个 Tauri 事件监听器', async () => {
    const wrapper = mount(EventApp)
    await flushPromises()

    expect(mockListen).toHaveBeenCalledTimes(3)
    expect(mockListen).toHaveBeenCalledWith('navigate', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('poll-completed', expect.any(Function))
    expect(mockListen).toHaveBeenCalledWith('release-state-changed', expect.any(Function))
    expect(wrapper.emitted('events-registered')![0]).toEqual([3])
  })

  it('在 onUnmounted 中清理所有监听器', async () => {
    const wrapper = mount(EventApp)
    await flushPromises()

    expect(mockUnlisten).not.toHaveBeenCalled()

    wrapper.unmount()

    // 每个事件监听器对应的 unlisten 都应被调用一次
    expect(mockUnlisten).toHaveBeenCalledTimes(3)
  })

  it('Vite HMR 模拟：重复挂载/卸载不会累积监听器', async () => {
    const wrapper1 = mount(EventApp)
    await flushPromises()
    wrapper1.unmount()

    expect(mockUnlisten).toHaveBeenCalledTimes(3)

    const wrapper2 = mount(EventApp)
    await flushPromises()

    // 第一次挂载时的 3 个 + 第二次挂载时的 3 个 = 6 次 listen 调用
    expect(mockListen).toHaveBeenCalledTimes(6)

    wrapper2.unmount()
    expect(mockUnlisten).toHaveBeenCalledTimes(6)

    // 验证最终的调用次数：3 listen 和 3 unlisten 每组，共 2 组
    expect(mockListen).toHaveBeenCalledTimes(6)
    expect(mockUnlisten).toHaveBeenCalledTimes(6)
  })
})
