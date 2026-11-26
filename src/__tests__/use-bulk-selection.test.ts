import { describe, it, expect, vi } from 'vitest'
import { computed, defineComponent, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { useBulkSelection } from '../composables/useBulkSelection'
import { hasActiveOverlay } from '../composables/contextMenuBus'

function mountBulk(opts?: {
  initialIds?: number[]
  canSelect?: boolean
  active?: boolean
  onEscapeBeforeExit?: () => boolean
}) {
  const ids = ref(opts?.initialIds ?? [1, 2, 3, 4])
  const canSelect = ref(opts?.canSelect ?? true)
  const active = ref(opts?.active ?? true)
  let api!: ReturnType<typeof useBulkSelection<number>>

  const Comp = defineComponent({
    setup() {
      api = useBulkSelection<number>({
        visibleIds: computed(() => ids.value),
        canSelect: computed(() => canSelect.value),
        active: computed(() => active.value),
        onEscapeBeforeExit: opts?.onEscapeBeforeExit,
      })
      return {}
    },
    template: '<div />',
  })

  const wrapper = mount(Comp)
  return { wrapper, ids, canSelect, active, api }
}

describe('useBulkSelection', () => {
  it('支持单选、Shift 区间连选、全选与清空', () => {
    const { wrapper, api } = mountBulk()
    expect(api.toggleSelectionMode()).toBe(true)
    expect(api.selectionMode.value).toBe(true)

    api.toggle(1)
    api.toggle(3, true)
    expect([...api.selectedIds.value]).toEqual([1, 2, 3])
    expect(api.selectedCount.value).toBe(3)
    expect(api.allVisibleSelected.value).toBe(false)

    api.selectAllVisible()
    expect(api.allVisibleSelected.value).toBe(true)

    api.clearSelected()
    expect(api.selectedCount.value).toBe(0)
    wrapper.unmount()
  })

  it('可见列表变化时自动剔除已不可见的选中项', async () => {
    const { wrapper, ids, api } = mountBulk()
    api.toggleSelectionMode()
    api.toggle(1)
    api.toggle(3)

    ids.value = [1, 2, 4]
    await nextTick()

    expect([...api.selectedIds.value]).toEqual([1])
    wrapper.unmount()
  })

  it('canSelect 变为 false 时自动退出选择模式', async () => {
    const { wrapper, canSelect, api } = mountBulk()
    api.toggleSelectionMode()
    api.toggle(2)

    canSelect.value = false
    await nextTick()

    expect(api.selectionMode.value).toBe(false)
    expect(api.selectedCount.value).toBe(0)
    expect(api.toggleSelectionMode()).toBe(false)
    wrapper.unmount()
  })

  it('选择模式下注册覆盖层，按 Esc 逐层退出并在非激活时忽略', () => {
    let popoverOpen = true
    const onEscapeBeforeExit = vi.fn(() => {
      if (!popoverOpen) return false
      popoverOpen = false
      return true
    })
    const { wrapper, active, api } = mountBulk({ onEscapeBeforeExit })

    expect(hasActiveOverlay()).toBe(false)
    api.toggleSelectionMode()
    expect(hasActiveOverlay()).toBe(true)

    active.value = false
    expect(hasActiveOverlay()).toBe(false)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    expect(api.selectionMode.value).toBe(true)

    active.value = true
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    expect(onEscapeBeforeExit).toHaveBeenCalledTimes(1)
    expect(api.selectionMode.value).toBe(true)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    expect(api.selectionMode.value).toBe(false)
    expect(hasActiveOverlay()).toBe(false)
    wrapper.unmount()
  })
})
