import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { enableAutoUnmount, mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import ReleaseSimpleList from '../components/ReleaseSimpleList.vue'
import { createRelease } from './helpers'
import type { ReleaseInfo } from '../api/releases'

// j/k 光标的真实实现侧：选中项标记、边界回卷、Enter 打开的资格判定。
const ReleaseItemStub = defineComponent({
  name: 'ReleaseItem',
  props: ['release', 'highlighted', 'selected'],
  emits: ['update', 'open-detail'],
  template:
    '<div class="stub-item" :class="{ sel: selected }" @click="$emit(\'open-detail\', release)">{{ release.tag_name }}</div>',
})

function mountList(releases: ReleaseInfo[]): VueWrapper {
  return mount(ReleaseSimpleList, {
    props: { releases, isFiltering: false, hasSearchQuery: false, deepSearch: false },
    global: { stubs: { ReleaseItem: ReleaseItemStub, ContextMenu: true } },
  }) as unknown as VueWrapper
}

// 列表按发布时间倒序：id 越大越新，越靠前。只有 v3 带正文（Enter 打开的资格判定）
function threeReleases(): ReleaseInfo[] {
  return [
    createRelease({ id: 1, tag_name: 'v1', published_at: '2025-06-01T00:00:00Z' }),
    createRelease({ id: 3, tag_name: 'v3', published_at: '2025-06-03T00:00:00Z', body: '正文' }),
    createRelease({ id: 2, tag_name: 'v2', published_at: '2025-06-02T00:00:00Z' }),
  ]
}

function handle(wrapper: VueWrapper) {
  return wrapper.vm as unknown as {
    moveSelection: (delta: number) => void
    selectFirst: () => boolean
    openSelected: () => void
  }
}

function selectedTags(wrapper: VueWrapper) {
  return wrapper.findAll('.stub-item.sel').map(n => n.text())
}

beforeEach(() => {
  vi.clearAllMocks()
  // jsdom 没有 scrollIntoView：虚拟列表定位选中项时会调用它
  Element.prototype.scrollIntoView = vi.fn()
})

enableAutoUnmount(afterEach)

describe('ReleaseSimpleList.vue — j/k 光标', () => {
  it('首次向下落在最新一条，继续向下逐条下移，首尾不回卷', async () => {
    const wrapper = mountList(threeReleases())

    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v3'])

    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v2'])

    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v1'])

    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v1'])
  })

  it('首次向上落在最旧一条', async () => {
    const wrapper = mountList(threeReleases())

    handle(wrapper).moveSelection(-1)
    await nextTick()

    expect(selectedTags(wrapper)).toEqual(['v1'])
  })

  it('空列表不产生光标', async () => {
    const wrapper = mountList([])

    handle(wrapper).moveSelection(1)
    await nextTick()

    expect(selectedTags(wrapper)).toEqual([])
    expect(handle(wrapper).selectFirst()).toBe(false)
  })

  it('Enter 只在选中项有正文/译文时打开详情，并携带全局时间序序列', async () => {
    const wrapper = mountList(threeReleases())

    handle(wrapper).moveSelection(1)
    handle(wrapper).openSelected()
    await nextTick()
    let events = wrapper.emitted('open-detail')!
    expect(events).toHaveLength(1)
    expect((events[0][0] as ReleaseInfo).tag_name).toBe('v3')
    expect((events[0][1] as ReleaseInfo[]).map(r => r.tag_name)).toEqual(['v3', 'v2', 'v1'])

    // v2 无正文：不打开
    handle(wrapper).moveSelection(1)
    handle(wrapper).openSelected()
    await nextTick()
    events = wrapper.emitted('open-detail')!
    expect(events).toHaveLength(1)
  })

  it('未选中时 Enter 不打开详情', () => {
    const wrapper = mountList(threeReleases())

    handle(wrapper).openSelected()

    expect(wrapper.emitted('open-detail')).toBeUndefined()
  })

  it('插入了更新的条目后，光标仍停在同一 release 上', async () => {
    const wrapper = mountList(threeReleases())
    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v3'])

    // 轮询拉来一条更新的：它插到序列最前，下标 0 换了人
    await wrapper.setProps({
      releases: [...threeReleases(), createRelease({ id: 4, tag_name: 'v4', published_at: '2025-06-04T00:00:00Z' })],
    })
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v3'])

    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v2'])
  })

  it('选中项被筛掉后光标不残留，筛选恢复时回到原条目', async () => {
    const wrapper = mountList(threeReleases())
    handle(wrapper).moveSelection(1)
    handle(wrapper).moveSelection(1)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v2'])

    await wrapper.setProps({ releases: [threeReleases()[1]] })
    await nextTick()
    expect(selectedTags(wrapper)).toEqual([])

    await wrapper.setProps({ releases: threeReleases() })
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v2'])
  })

  it('列表数据变化后光标不越界', async () => {
    const wrapper = mountList(threeReleases())
    handle(wrapper).moveSelection(1)
    handle(wrapper).moveSelection(1)
    await wrapper.setProps({ releases: [createRelease({ id: 9, tag_name: 'only' })] })

    handle(wrapper).moveSelection(1)
    await nextTick()

    expect(selectedTags(wrapper)).toEqual(['only'])
  })

  it('selectFirst 落在首条并返回是否成功', async () => {
    const wrapper = mountList(threeReleases())

    expect(handle(wrapper).selectFirst()).toBe(true)
    await nextTick()
    expect(selectedTags(wrapper)).toEqual(['v3'])
  })

  it('点击卡片打开详情仍按全局时间序导航（与光标无关）', async () => {
    const wrapper = mountList(threeReleases())

    await wrapper.findAll('.stub-item')[1].trigger('click')

    const events = wrapper.emitted('open-detail')!
    expect((events[0][0] as ReleaseInfo).tag_name).toBe('v2')
    expect((events[0][1] as ReleaseInfo[]).map(r => r.id)).toEqual([3, 2, 1])
  })
})
