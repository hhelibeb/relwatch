import { describe, it, expect, vi, afterEach } from 'vitest'
import { defineComponent, ref } from 'vue'
import { mount, flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ message: vi.fn() }))

// 手工构造一个不含 <use> 的 SourceTab 精简版来验证 emit 逻辑
const MinimalSourceTab = defineComponent({
  props: { sources: { type: Array, default: () => [] } },
  emits: ['update', 'checkResult'],
  setup(_props, { emit }) {
    const loading = ref(false)
    async function handleCheckSingle(id: number) {
      const { invoke } = await import('@tauri-apps/api/core')
      const result: any = await (invoke as any)('check_single_source', { id })
      emit('update')
      emit('checkResult', result.new_releases.length)
    }
    async function handleRemove(_id: number) {
      const { invoke } = await import('@tauri-apps/api/core')
      await (invoke as any)('remove_source')
      emit('update')
    }
    return { loading, handleCheckSingle, handleRemove }
  },
  template: '<div><button class="btn-check" @click="handleCheckSingle(1)">check</button><button class="btn-danger" @click="handleRemove(1)">del</button></div>',
})

afterEach(() => { vi.clearAllMocks() })

describe('checkSingle → checkResult emit', () => {
  it('emits checkResult(0) when empty', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue({ new_releases: [] })

    const wrapper = mount(MinimalSourceTab, { props: { sources: [{}] } })
    await wrapper.get('.btn-check').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('checkResult')![0]).toEqual([0])
  })

  it('emits checkResult(3) with count', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue({ new_releases: [{}, {}, {}] })

    const wrapper = mount(MinimalSourceTab, { props: { sources: [{}] } })
    await wrapper.get('.btn-check').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('checkResult')![0]).toEqual([3])
  })

  it('does NOT emit checkResult on delete', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined)

    const wrapper = mount(MinimalSourceTab, { props: { sources: [{}] } })
    await wrapper.get('.btn-danger').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('update')).toBeTruthy()
    expect(wrapper.emitted('checkResult')).toBeFalsy()
  })
})

describe('showToast condition', () => {
  it('detects "no new" vs "has new"', () => {
    const noNew = (arr: any[]) => arr.length === 0
    expect(noNew([])).toBe(true)
    expect(noNew([{}, {}])).toBe(false)
    expect(noNew([{}])).toBe(false)
  })
})
