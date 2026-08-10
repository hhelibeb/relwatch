import { describe, it, expect, vi, beforeEach } from 'vitest'
import { shallowMount } from '@vue/test-utils'
import ReleaseAggregatedList from '../components/ReleaseAggregatedList.vue'
import type { ReleaseInfo } from '../api/releases'
import type { RepoGroup } from '../components/releaseTypes'

// 阶段 2-1：聚合列表（ReleaseAggregatedList.vue）repoGroups 分组逻辑专项测试。
// 覆盖：空/单/多 repo 聚合、组内按发布时间降序、组间按最新版本降序、
// 分组键含 source_type（不同源类型同 owner/repo 不串源）、
// 展开/折叠/全展开、open-detail 携带组内序列、分组显示名（注册表 displayName）。

vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
}))

vi.mock('../composables/useUsageTracking', () => ({
  track: vi.fn(),
}))

vi.mock('../utils', () => ({
  formatDate: vi.fn(() => '2025-06-01'),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))

vi.mock('../composables/useContextMenu', () => ({
  useContextMenu: vi.fn(() => ({
    contextMenu: null,
    closeContextMenu: vi.fn(),
    handleContextMenu: vi.fn(),
    handleCopyLink: vi.fn(),
    handleOpenLink: vi.fn(),
  })),
}))

function createRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
  return {
    id: 1,
    source_id: 1,
    source_type: 'github',
    owner: 'tauri-apps',
    repo: 'tauri',
    tag_name: 'v2.0.0',
    release_name: 'Tauri 2.0 Stable',
    html_url: 'https://github.com/tauri-apps/tauri/releases/tag/v2.0.0',
    published_at: '2025-06-01T00:00:00Z',
    prerelease: false,
    body: null,
    detected_at: '2025-06-01T00:00:00Z',
    notification_status: 'pending',
    snooze_until: null,
    ai_summary: null,
    ai_importance: null,
    body_translated: null,
    extra_metadata: null,
    source_description: null,
    ...overrides,
  }
}

function mountList(releases: ReleaseInfo[], isFiltering = false) {
  return shallowMount(ReleaseAggregatedList, { props: { releases, isFiltering } })
}

function groups(wrapper: ReturnType<typeof mountList>): RepoGroup[] {
  return (wrapper.vm as unknown as { repoGroups: RepoGroup[] }).repoGroups
}

describe('ReleaseAggregatedList.vue — 分组聚合', () => {
  it('空数据：无分组，显示 empty 文案（过滤时 no_match）', () => {
    const wrapper = mountList([])
    expect(groups(wrapper)).toEqual([])
    expect(wrapper.text()).toContain('release.empty')

    const filtered = mountList([], true)
    expect(filtered.text()).toContain('release.no_match')
  })

  it('单 repo 多版本 → 一组，组内按发布时间降序', () => {
    const releases = [
      createRelease({ id: 1, tag_name: 'v1.0.0', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, tag_name: 'v1.1.0', published_at: '2025-06-10T00:00:00Z' }),
      createRelease({ id: 3, tag_name: 'v0.9.0', published_at: '2025-05-01T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)
    const g = groups(wrapper)

    expect(g).toHaveLength(1)
    expect(g[0].releases.map(r => r.tag_name)).toEqual(['v1.1.0', 'v1.0.0', 'v0.9.0'])
    // 组头显示最新 tag
    expect(wrapper.text()).toContain('v1.1.0')
  })

  it('多 repo → 组按最新版本时间降序', () => {
    const releases = [
      createRelease({ id: 1, owner: 'a', repo: 'old', published_at: '2025-05-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'b', repo: 'new', published_at: '2025-06-20T00:00:00Z' }),
      createRelease({ id: 3, owner: 'c', repo: 'mid', published_at: '2025-06-01T00:00:00Z' }),
    ]
    const g = groups(mountList(releases))

    // 分组键带 source_type 前缀（sourceRepoKey：type|owner|repo 小写）
    expect(g.map(x => x.key)).toEqual(['github|b|new', 'github|c|mid', 'github|a|old'])
  })

  it('分组键含 source_type：不同源类型同 owner/repo 不串源', () => {
    const releases = [
      createRelease({ id: 1, source_type: 'github', owner: 'same', repo: 'x', tag_name: 'v1', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, source_type: 'youtube', owner: 'same', repo: 'x', tag_name: 'vid1', published_at: '2025-06-02T00:00:00Z' }),
    ]
    const g = groups(mountList(releases))

    expect(g).toHaveLength(2)
    expect(g.map(x => x.key)).toEqual(['youtube|same|x', 'github|same|x'])
  })

  it('分组键大小写不敏感（owner/repo 统一小写）', () => {
    const releases = [
      createRelease({ id: 1, owner: 'Owner', repo: 'Repo', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'owner', repo: 'repo', published_at: '2025-06-02T00:00:00Z' }),
    ]
    const g = groups(mountList(releases))

    expect(g).toHaveLength(1)
    expect(g[0].key).toBe('github|owner|repo')
  })
})

describe('ReleaseAggregatedList.vue — 展开/折叠', () => {
  it('默认全部折叠：组体不渲染', () => {
    const releases = [createRelease({ id: 1, published_at: '2025-06-01T00:00:00Z' })]
    const wrapper = mountList(releases)

    expect((wrapper.vm as any).allExpanded).toBe(false)
    expect(wrapper.find('.repo-group-body').exists()).toBe(false)
  })

  it('toggleRepo 展开后渲染组内条目，再点收起', async () => {
    const releases = [
      createRelease({ id: 1, published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, published_at: '2025-06-02T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)
    const key = groups(wrapper)[0].key

    await wrapper.find('.repo-group-toggle').trigger('click')
    expect((wrapper.vm as any).expandedRepos.has(key)).toBe(true)
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(1)

    await wrapper.find('.repo-group-toggle').trigger('click')
    expect((wrapper.vm as any).expandedRepos.has(key)).toBe(false)
  })

  it('expandAll / toggleAllRepos 全展开与收起', async () => {
    const releases = [
      createRelease({ id: 1, owner: 'a', repo: 'r1', published_at: '2025-06-01T00:00:00Z' }),
      createRelease({ id: 2, owner: 'b', repo: 'r2', published_at: '2025-06-02T00:00:00Z' }),
    ]
    const wrapper = mountList(releases)

    await wrapper.find('.repo-toolbar .btn-sm').trigger('click')
    expect((wrapper.vm as any).allExpanded).toBe(true)
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(2)

    await wrapper.find('.repo-toolbar .btn-sm').trigger('click')
    expect((wrapper.vm as any).allExpanded).toBe(false)
    expect(wrapper.findAll('.repo-group-body')).toHaveLength(0)
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
    const g = groups(wrapper)
    const groupA = g.find(x => x.key === 'github|a|r1')!

    ;(wrapper.vm as any).forwardOpenDetail(groupA.releases[0])

    const events = wrapper.emitted('open-detail')!
    expect(events).toBeTruthy()
    expect(events[0][0]).toEqual(groupA.releases[0])
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
