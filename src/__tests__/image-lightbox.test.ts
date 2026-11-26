import { mount } from '@vue/test-utils'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { nextTick } from 'vue'
import ImageLightbox from '../components/common/ImageLightbox.vue'
import { openReleaseUrl } from '../api/client'
import { t } from '../i18n'

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))

const openUrlMock = vi.mocked(openReleaseUrl)

async function mountLightbox() {
  const wrapper = mount(ImageLightbox, {
    props: { src: 'http://media.localhost/x', originalUrl: 'https://example.com/x.png' },
    global: { stubs: { teleport: true } },
    attachTo: document.body,
  })
  // visible 在 onMounted 置位，stage 下一帧才渲染
  await nextTick()
  return wrapper
}

// 关闭走「先播离场动画再 emit」：等 240ms 延迟落地
function waitClose() {
  return new Promise(r => setTimeout(r, 280))
}

beforeEach(() => {
  openUrlMock.mockClear()
})

describe('ImageLightbox', () => {
  it('初始为适应窗口（100%）', async () => {
    const wrapper = await mountLightbox()
    expect(wrapper.find('.lightbox-percent').text()).toBe('100%')
    wrapper.unmount()
  })

  it('放大/缩小按钮改变缩放比例，适应窗口复位', async () => {
    const wrapper = await mountLightbox()
    await wrapper.find(`button[title="${t('lightbox.zoom_in')}"]`).trigger('click')
    expect(wrapper.find('.lightbox-percent').text()).toBe('125%')
    await wrapper.find(`button[title="${t('lightbox.zoom_out')}"]`).trigger('click')
    expect(wrapper.find('.lightbox-percent').text()).toBe('100%')
    await wrapper.find(`button[title="${t('lightbox.zoom_in')}"]`).trigger('click')
    await wrapper.find(`button[title="${t('lightbox.zoom_reset')}"]`).trigger('click')
    expect(wrapper.find('.lightbox-percent').text()).toBe('100%')
    wrapper.unmount()
  })

  it('双击在 100% 与 250% 之间切换', async () => {
    const wrapper = await mountLightbox()
    await wrapper.find('.lightbox-stage').trigger('dblclick', { clientX: 100, clientY: 100 })
    expect(wrapper.find('.lightbox-percent').text()).toBe('250%')
    await wrapper.find('.lightbox-stage').trigger('dblclick', { clientX: 100, clientY: 100 })
    expect(wrapper.find('.lightbox-percent').text()).toBe('100%')
    wrapper.unmount()
  })

  it('旋转按钮每次顺时针 90°，适应窗口一并归零', async () => {
    const wrapper = await mountLightbox()
    const rotateBtn = wrapper.find(`button[title="${t('lightbox.rotate')}"]`)
    const styleOf = () => wrapper.find('.lightbox-img').attributes('style') ?? ''
    expect(styleOf()).toContain('rotate(0deg)')
    await rotateBtn.trigger('click')
    expect(styleOf()).toContain('rotate(90deg)')
    expect(wrapper.find('.lightbox-img').classes()).toContain('rotated')
    await rotateBtn.trigger('click')
    await rotateBtn.trigger('click')
    await rotateBtn.trigger('click')
    expect(styleOf()).toContain('rotate(0deg)')
    expect(wrapper.find('.lightbox-img').classes()).not.toContain('rotated')
    await rotateBtn.trigger('click')
    await wrapper.find(`button[title="${t('lightbox.zoom_reset')}"]`).trigger('click')
    expect(styleOf()).toContain('rotate(0deg)')
    wrapper.unmount()
  })

  it('R 键顺时针旋转并吞掉事件，Ctrl+R 不接管', async () => {
    const wrapper = await mountLightbox()
    const styleOf = () => wrapper.find('.lightbox-img').attributes('style') ?? ''
    const plain = new KeyboardEvent('keydown', { key: 'r', cancelable: true })
    window.dispatchEvent(plain)
    expect(plain.defaultPrevented).toBe(true)
    await nextTick()
    expect(styleOf()).toContain('rotate(90deg)')
    const withCtrl = new KeyboardEvent('keydown', { key: 'r', ctrlKey: true, cancelable: true })
    window.dispatchEvent(withCtrl)
    await nextTick()
    expect(withCtrl.defaultPrevented).toBe(false)
    expect(styleOf()).toContain('rotate(90deg)')
    wrapper.unmount()
  })

  it('Alt+R / Meta+R 同样不接管', async () => {
    const wrapper = await mountLightbox()
    const styleOf = () => wrapper.find('.lightbox-img').attributes('style') ?? ''
    for (const mods of [{ altKey: true }, { metaKey: true }]) {
      const e = new KeyboardEvent('keydown', { key: 'r', cancelable: true, ...mods })
      window.dispatchEvent(e)
      expect(e.defaultPrevented).toBe(false)
    }
    await nextTick()
    expect(styleOf()).toContain('rotate(0deg)')
    wrapper.unmount()
  })

  it('180° 不加 rotated 类，270° 加', async () => {
    const wrapper = await mountLightbox()
    const rotateBtn = wrapper.find(`button[title="${t('lightbox.rotate')}"]`)
    const img = () => wrapper.find('.lightbox-img')
    await rotateBtn.trigger('click')
    expect(img().classes()).toContain('rotated')
    await rotateBtn.trigger('click')
    expect(img().classes()).not.toContain('rotated')
    expect(img().attributes('style')).toContain('rotate(180deg)')
    await rotateBtn.trigger('click')
    expect(img().classes()).toContain('rotated')
    expect(img().attributes('style')).toContain('rotate(270deg)')
    wrapper.unmount()
  })

  it('旋转后双击缩回 100% 不清零角度', async () => {
    const wrapper = await mountLightbox()
    const stage = wrapper.find('.lightbox-stage')
    await wrapper.find(`button[title="${t('lightbox.rotate')}"]`).trigger('click')
    await stage.trigger('dblclick', { clientX: 100, clientY: 100 })
    expect(wrapper.find('.lightbox-percent').text()).toBe('250%')
    await stage.trigger('dblclick', { clientX: 100, clientY: 100 })
    expect(wrapper.find('.lightbox-percent').text()).toBe('100%')
    expect(wrapper.find('.lightbox-img').attributes('style')).toContain('rotate(90deg)')
    wrapper.unmount()
  })

  it('图片加载失败时缩放与旋转按钮禁用', async () => {
    const wrapper = await mountLightbox()
    await wrapper.find('.lightbox-img').trigger('error')
    await nextTick()
    expect(wrapper.find('.lightbox-error').exists()).toBe(true)
    for (const key of ['lightbox.zoom_in', 'lightbox.zoom_out', 'lightbox.zoom_reset', 'lightbox.rotate']) {
      expect(wrapper.find(`button[title="${t(key)}"]`).attributes('disabled')).toBeDefined()
    }
    wrapper.unmount()
  })

  it('Esc 关闭且不穿透（defaultPrevented）', async () => {
    const wrapper = await mountLightbox()
    const e = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    window.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
    await waitClose()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('方向键被拦截，不触发弹窗版本导航', async () => {
    const wrapper = await mountLightbox()
    const e = new KeyboardEvent('keydown', { key: 'ArrowRight', cancelable: true })
    window.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
    expect(wrapper.emitted('close')).toBeUndefined()
    wrapper.unmount()
  })

  it('点击空白关闭，点击图片不关闭', async () => {
    const wrapper = await mountLightbox()
    const stage = wrapper.find('.lightbox-stage')
    await stage.trigger('pointerdown')
    await stage.trigger('pointerup')
    await waitClose()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()

    const second = await mountLightbox()
    const img = second.find('.lightbox-img')
    await img.trigger('pointerdown')
    await second.find('.lightbox-stage').trigger('pointerup')
    await waitClose()
    expect(second.emitted('close')).toBeUndefined()
    second.unmount()
  })

  it('打开原图走系统浏览器', async () => {
    const wrapper = await mountLightbox()
    await wrapper.find(`button[title="${t('lightbox.open_original')}"]`).trigger('click')
    expect(openUrlMock).toHaveBeenCalledWith('https://example.com/x.png')
    wrapper.unmount()
  })

  it('卸载后键盘监听不再响应', async () => {
    const wrapper = await mountLightbox()
    wrapper.unmount()
    const e = new KeyboardEvent('keydown', { key: 'Escape', cancelable: true })
    window.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(false)
  })
})
