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

// 摘要显示层被行级截断（真实环境由 useLineClamp 按布局算出）：用「前 20 字 + 省略号」模拟
vi.mock('../composables/useLineClamp', async () => {
  const { ref, computed } = await import('vue')
  return {
    useLineClamp: (opts: { text: () => string | null }) => ({
      textRef: ref(null),
      display: computed(() => {
        const text = opts.text()
        return text ? `${text.slice(0, 20)}…` : null
      }),
      hasThirdLine: ref(true),
    }),
  }
})

describe('ReleaseItem 摘要截断后的命中提示', () => {
  it('命中只落在折叠区时，用完整摘要给出命中上下文', () => {
    const release = createRelease({
      ai_summary: '开头'.repeat(80) + 'kernel' + '结尾'.repeat(80),
      body: '本次更新调整了若干实现细节。',
    })
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel' } })
    const snippet = wrapper.find('.release-hit-snippet')
    expect(snippet.exists()).toBe(true)
    expect(snippet.find('mark.search-hit').text()).toBe('kernel')
    expect(snippet.text()).toContain(t('release.search_hit_body'))
  })

  it('命中在摘要可见部分时不出片段', () => {
    const release = createRelease({
      ai_summary: '修复 kernel 崩溃',
      body: '本次更新调整了若干实现细节。',
    })
    const wrapper = mount(ReleaseItem, { props: { release, searchQuery: 'kernel' } })
    expect(wrapper.find('.release-hit-snippet').exists()).toBe(false)
  })
})
