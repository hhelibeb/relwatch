import { mount } from '@vue/test-utils'
import { describe, it, expect, vi } from 'vitest'
import ReleaseItem from '../components/ReleaseItem.vue'
import { createRelease } from './helpers'
import { t } from '../i18n'

vi.mock('../api/releases', () => ({
  setNotificationState: vi.fn(),
  deleteRelease: vi.fn(),
  translateRelease: vi.fn(),
  setReleaseFlag: vi.fn(),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  copyTextToClipboard: vi.fn(),
  copyImageToClipboard: vi.fn(),
}))

describe('ReleaseItem 搜索命中', () => {
  it('可见预览不含命中词时展示正文命中片段', () => {
    const release = createRelease({
      ai_summary: '一段与查询无关的摘要',
      body: '本次更新修复了 kernel 崩溃问题，并优化了启动速度。',
    })
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel' } })
    const snippet = wrapper.find('.release-hit-snippet')
    expect(snippet.exists()).toBe(true)
    expect(snippet.find('mark.search-hit').text()).toBe('kernel')
    expect(snippet.text()).toContain(t('release.search_hit_body'))
  })

  it('摘要已含命中词时不展示片段（高亮本身可见）', () => {
    const release = createRelease({
      ai_summary: '修复 kernel 崩溃',
      body: 'kernel details',
    })
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel' } })
    expect(wrapper.find('.release-hit-snippet').exists()).toBe(false)
  })

  it('无搜索词时不展示片段', () => {
    const release = createRelease({ body: 'kernel' })
    const wrapper = mount(ReleaseItem, { props: { release } })
    expect(wrapper.find('.release-hit-snippet').exists()).toBe(false)
  })

  it('目录预览未命中时回退到深度索引全文', () => {
    const release = createRelease({ ai_summary: '无关摘要', body: '短预览不含命中词' })
    const bodyIndex = new Map([[1, ['deep body contains kernel fix', '']]])
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel', bodyIndex } })
    const snippet = wrapper.find('.release-hit-snippet')
    expect(snippet.exists()).toBe(true)
    expect(snippet.find('mark.search-hit').text()).toBe('kernel')
  })

  it('点击片段打开详情', async () => {
    const release = createRelease({ ai_summary: '无关摘要', body: 'kernel fix' })
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel' } })
    await wrapper.find('.release-hit-snippet').trigger('click')
    expect(wrapper.emitted('open-detail')).toHaveLength(1)
  })

  it('标题/仓库名中的命中词也会被高亮收集（不改动 DOM）', async () => {
    const release = createRelease({ repo: 'kernel-tools', ai_summary: '摘要', body: null })
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel' } })
    await new Promise(r => setTimeout(r, 0))
    // 高亮走 CSS Highlight API：DOM 保持单一文本节点，不被 <mark> 拆分
    const repo = wrapper.find('.release-repo')
    expect(repo.element.childNodes).toHaveLength(1)
    expect(repo.text()).toBe('tauri-apps/kernel-tools')
  })
})