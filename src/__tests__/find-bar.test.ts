import { describe, it, expect, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import FindBar from '../components/common/FindBar.vue'

describe('FindBar', () => {
  it('展示命中计数、无匹配状态，并正确转发按钮与输入事件', async () => {
    const setInputEl = vi.fn()
    const wrapper = mount(FindBar, {
      props: {
        modelValue: 'foo',
        matchCount: 3,
        currentIndex: 1,
        placeholder: '查找',
        prevLabel: '上一个',
        nextLabel: '下一个',
        closeLabel: '关闭',
        classPrefix: 'demo-find',
        compact: true,
        setInputEl,
      },
    })

    expect(setInputEl).toHaveBeenCalledWith(wrapper.get('.demo-find-input').element)
    expect(wrapper.get('.demo-find-count').text()).toBe('2 / 3')
    expect(wrapper.get('.demo-find-count').classes()).not.toContain('no-match')

    const btns = wrapper.findAll('.demo-find-btn')
    expect(btns).toHaveLength(3)
    await btns[0].trigger('click')
    await btns[1].trigger('click')
    await btns[2].trigger('click')
    expect(wrapper.emitted('prev')).toHaveLength(1)
    expect(wrapper.emitted('next')).toHaveLength(1)
    expect(wrapper.emitted('close')).toHaveLength(1)

    wrapper.unmount()
    expect(setInputEl).toHaveBeenLastCalledWith(null)
  })

  it('无匹配时展示 0 / 0、no-match 类并禁用前后跳转按钮', () => {
    const wrapper = mount(FindBar, {
      props: {
        modelValue: 'foo',
        matchCount: 0,
        currentIndex: -1,
        placeholder: '查找',
        prevLabel: '上一个',
        nextLabel: '下一个',
        closeLabel: '关闭',
        classPrefix: 'demo-find',
      },
    })

    const btns = wrapper.findAll('.demo-find-btn')
    expect(wrapper.get('.demo-find-count').text()).toBe('0 / 0')
    expect(wrapper.get('.demo-find-count').classes()).toContain('no-match')
    expect((btns[0].element as HTMLButtonElement).disabled).toBe(true)
    expect((btns[1].element as HTMLButtonElement).disabled).toBe(true)
    wrapper.unmount()
  })
})
