import { describe, it, expect, beforeEach } from 'vitest'
import {
  loadReleaseViewState,
  saveReleaseViewState,
  loadExpandedRepos,
  saveExpandedRepos,
  defaultReleaseViewState,
} from '../utils/releaseViewStorage'

const VIEW_KEY = 'relwatch.release.view'
const EXPANDED_KEY = 'relwatch.release.expandedRepos'

beforeEach(() => localStorage.clear())

describe('releaseViewStorage — 视图状态', () => {
  it('无存储 / 损坏 JSON / 非对象：一律回退默认值，不抛异常', () => {
    expect(loadReleaseViewState()).toEqual(defaultReleaseViewState)
    localStorage.setItem(VIEW_KEY, '{not json')
    expect(loadReleaseViewState()).toEqual(defaultReleaseViewState)
    localStorage.setItem(VIEW_KEY, '"a string"')
    expect(loadReleaseViewState()).toEqual(defaultReleaseViewState)
    localStorage.setItem(VIEW_KEY, 'null')
    expect(loadReleaseViewState()).toEqual(defaultReleaseViewState)
  })

  it('逐字段校验：非法值单独回退，合法字段保留', () => {
    localStorage.setItem(VIEW_KEY, JSON.stringify({ viewMode: 'calendar', statusFilter: 'nope', flagFilter: 3 }))
    const state = loadReleaseViewState()
    expect(state.viewMode).toBe('calendar')
    expect(state.statusFilter).toBe('all')
    expect(state.flagFilter).toBe(3)
    expect(state.versionFilter).toBe('all')
  })

  it('写入后读回一致（往返）', () => {
    const state = { ...defaultReleaseViewState, viewMode: 'aggregated' as const, importanceFilter: '大' as const, sourceFilter: 'youtube' as const }
    saveReleaseViewState(state)
    expect(loadReleaseViewState()).toEqual(state)
  })

  it('旗标颜色索引 1-6 与 flagged/unflagged 都是合法值', () => {
    for (const flagFilter of [1, 2, 3, 4, 5, 6, 'flagged', 'unflagged'] as const) {
      saveReleaseViewState({ ...defaultReleaseViewState, flagFilter })
      expect(loadReleaseViewState().flagFilter).toBe(flagFilter)
    }
  })
})

describe('releaseViewStorage — 聚合展开分组', () => {
  it('无存储 / 非数组：空集合', () => {
    expect(loadExpandedRepos()).toEqual(new Set())
    localStorage.setItem(EXPANDED_KEY, '{"a":1}')
    expect(loadExpandedRepos()).toEqual(new Set())
  })

  it('往返一致，且过滤非字符串项', () => {
    saveExpandedRepos(new Set(['github|a|b', 'youtube|c|']))
    expect([...loadExpandedRepos()]).toEqual(['github|a|b', 'youtube|c|'])
    localStorage.setItem(EXPANDED_KEY, JSON.stringify(['ok', 1, null]))
    expect([...loadExpandedRepos()]).toEqual(['ok'])
  })

  it('超过上限只保留最近 500 项', () => {
    saveExpandedRepos(new Set(Array.from({ length: 600 }, (_, i) => `k${i}`)))
    const kept = loadExpandedRepos()
    expect(kept.size).toBe(500)
    expect(kept.has('k0')).toBe(false)
    expect(kept.has('k599')).toBe(true)
  })
})
