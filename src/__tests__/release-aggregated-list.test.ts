import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import ReleaseAggregatedList from '../components/ReleaseAggregatedList.vue'
import { t } from '../i18n'
import { createRelease } from './helpers'
import type { ReleaseInfo } from '../api/releases'

// API 边界：仅替换 openReleaseUrl；注册表 / formatDate / i18n 走真实实现
vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, openReleaseUrl: vi.fn() }
})
// 统计上报：内存聚合 + 5s 定时刷库，仅置空 track 避免定时器
vi.mock('../composables/useUsageTracking', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../composables/useUsageTracking')>()
  return { ...actual, track: vi.fn() }
})

// ReleaseItem 自定义 stub：通过点击事件入口驱动 open-detail（真实绑定链路）
const ReleaseItemStub = defineComponent({
  name: 'ReleaseItem',
  props: ['release'],
  emits: ['update', 'open-detail'],
  template: '<div class="stub-release-item" @click="$emit(\'open-detail\', release)">{{ release.tag_name }}</div>',
})

function mountList(releases: ReleaseInfo[], isFiltering = false) {
  return mount(ReleaseAggregatedList, {
    props: { releases, isFiltering },
    global: { stubs: { ReleaseItem: ReleaseItemStub, ContextMenu: true } },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
})

/**
 * ReleaseAggregatedList.vue 行为测试（真实分组逻辑 + 注册表显示名）
 * 覆盖：空/单/多 repo 聚合、组内按发布时间降序、组间按最新版本降序、
 * 分组键含 source_type、展开/折叠/全展开、open-detail 携带组内序列。
 */
describe('ReleaseAggregatedList.vue — 分组聚合', () => {
  it('空数据：无分组，显示 empty 文案（过滤时 no_match）', () => {
    const wrapper = mountList([])
    expect(wrapper.find('.empty').text()).toContain(t('release.empty'))
    expect(wrapper.findAll('.repo-group')).toHaveLength(0)

    const filtered = mountList([], true)
    expect(filtered.find('.empty').text()).toContain(t('release.no_match'))
  })

  it('单 repo 多版本 → 一组，组内按发布时间降序', async () => {
    const releases = [
      createRelease({ id: 1, tag_name: 'v1.0.0', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, tag_name: 'v1.1.0', published_at: '2025-06-10T00:00:00Z' }),
      createRelease({ id: 3, tag_name: 'v0.9.0', published_at: '2025-05-01T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    expect(wrapper.findAll('.repo-group')).toHaveLength(1)
    // 组头显示最新 tag
    expect(wrapper.find('.repo-latest-tag').text()).toBe('v1.1.0')

    // 展开后组内条目按发布时间降序
    await wrapper.find('.repo-group-toggle').trigger('click')
    const itemTags = wrapper.findAll('.stub-release-item').map(i => i.text())
    expect(itemTags).toEqual(['v1.1.0', 'v1.0.0', 'v0.9.0'])
  })

  it('多 repo → 组按最新版本时间降序', () => {
    const releases = [
      createRelease({ id: 1, owner: 'a', repo: 'old', published_at: '2025-05-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'b', repo: 'new', published_at: '2025-06-20T00:00:00Z' }),
      createRelease({ id: 3, owner: 'c', repo: 'mid', published_at: '2025-06-01T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    // 组按最新版本降序：new → mid → old
    expect(wrapper.findAll('.repo-name').map(n => n.text())).toEqual(['b/new', 'c/mid', 'a/old'])
  })

  it('分组键含 source_type：不同源类型同 owner/repo 不串源', () => {
    const releases = [
      createRelease({ id: 1, source_type: 'github', owner: 'same', repo: 'x', tag_name: 'v1', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, source_type: 'youtube', owner: 'same', repo: 'x', tag_name: 'vid1', published_at: '2025-06-02T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    // 两个独立分组（github|same|x 与 youtube|same|x）
    expect(wrapper.findAll('.repo-group')).toHaveLength(2)
  })

  it('分组键大小写不敏感（owner/repo 统一小写）', () => {
    const releases = [
      createRelease({ id: 1, owner: 'Owner', repo: 'Repo', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'owner', repo: 'repo', published_at: '2025-06-02T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    expect(wrapper.findAll('.repo-group')).toHaveLength(1)
    expect(wrapper.find('.repo-latest-tag').text()).toBe('v2.0.0')
  })
})

describe('ReleaseAggregatedList.vue — 展开/折叠', () => {
  it('默认全部折叠：组体不渲染', () => {
    const releases = [createRelease({ id: 1, published_at: '2025-06-01T00:00:00Z' })]
    const wrapper = mountList(releases)

    expect(wrapper.find('.repo-group-body').exists()).toBe(false)
  })

  it('点击组头展开后渲染组内条目，再点收起', async () => {
    const releases = [
      createRelease({ id: 1, published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, published_at: '2025-06-02T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    await wrapper.find('.repo-group-toggle').trigger('click')
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(1)
    expect(wrapper.findAll('.stub-release-item')).toHaveLength(2)

    await wrapper.find('.repo-group-toggle').trigger('click')
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(0)
  })

  it('全部展开/收起按钮切换所有分组', async () => {
    const releases = [
      createRelease({ id: 1, owner: 'a', repo: 'r1', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'b', repo: 'r2', published_at: '2025-06-02T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    await wrapper.find('.repo-toolbar .btn-sm').trigger('click')
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(2)
    expect(wrapper.find('.repo-toolbar .btn-sm').text()).toContain(t('release.collapse_all'))

    await wrapper.find('.repo-toolbar .btn-sm').trigger('click')
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(0)
    expect(wrapper.find('.repo-toolbar .btn-sm').text()).toContain(t('release.expand_all'))
  })
})

describe('ReleaseAggregatedList.vue — open-detail 导航序列', () => {
  it('open-detail 携带组内全部版本（同仓库内导航，不跨组）', async () => {
    const releases = [
      createRelease({ id: 1, owner: 'a', repo: 'r1', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'a', repo: 'r1', published_at: '2025-06-02T00:00:00Z' }),
      createRelease({ id: 3, owner: 'b', repo: 'r2', published_at: '2025-06-03T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    // 展开 a/r1 组（最新版本较旧，排第二），点击组内第一条（id 2，组内降序）
    await wrapper.findAll('.repo-group-toggle')[1].trigger('click')
    await wrapper.findAll('.stub-release-item')[0].trigger('click')

    const events = wrapper.emitted('open-detail')!
    expect(events).toBeTruthy()
    expect(events[0][0]).toMatchObject({ id: 2 })
    // 序列 = 组内全部（降序），不含 b|r2 的 release
    expect((events[0][1] as ReleaseInfo[]).map(r => r.id)).toEqual([2, 1])
  })
})

describe('ReleaseAggregatedList.vue — 分组显示名（注册表 displayName）', () => {
  it('youtube 显示频道名（source_description），不显示 channel_id', () => {
    const releases = [createRelease({
      source_type: 'youtube',
      owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw',
      repo: '',
      tag_name: 'vid1',
      source_description: '时局眼',
      published_at: '2025-06-01T00:00:00Z',
    })]
    const wrapper = mountList(releases)

    expect(wrapper.find('.repo-name').text()).toBe('时局眼')
    expect(wrapper.text()).not.toContain('UCXuqSBlHAE6Xw')
  })

  it('bilibili 显示 UP 主名（source_description），不显示 UID', () => {
    const releases = [createRelease({
      source_type: 'bilibili',
      owner: '476599099',
      repo: '',
      tag_name: 'BV1xx',
      source_description: '某UP主',
      published_at: '2025-06-01T00:00:00Z',
    })]
    const wrapper = mountList(releases)

    expect(wrapper.find('.repo-name').text()).toBe('某UP主')
    expect(wrapper.text()).not.toContain('476599099')
  })

  it('github 默认 owner/repo，无 displayName', () => {
    const releases = [createRelease({ owner: 'tauri-apps', repo: 'tauri', published_at: '2025-06-01T00:00:00Z' })]
    const wrapper = mountList(releases)

    expect(wrapper.find('.repo-name').text()).toBe('tauri-apps/tauri')
  })
})
