import { describe, it, expect, vi } from 'vitest'
import {
  formatDate,
  releaseMatchesSearch,
  tokenizeQuery,
  getSearchIndex,
  logLevelClass,
  statusLabel,
  statusClass,
  isUnreadStatus,
  isReadStatus,
  skillShortName,
} from '../utils'
import { sourceTypeDefs } from '../api/source-registry'

// ── releaseMatchesSearch ──────────────────────────────────────────

function makeRelease(overrides = {}) {
  return {
    owner: 'hhelibeb',
    repo: 'relwatch',
    tag_name: 'v1.0.0',
    release_name: 'RelWatch 1.0.0',
    body: null,
    source_type: 'github',      // ← 默认 GitHub，body 归 Tier2；视频源需显式指定
    ...overrides,
  }
}

describe('releaseMatchesSearch', () => {
  it('匹配完整的 owner/repo', () => {
    expect(releaseMatchesSearch(makeRelease(), 'hhelibeb/relwatch')).toBe(true)
    expect(releaseMatchesSearch(makeRelease({ owner: 'microsoft', repo: 'vscode' }), 'hhelibeb/relwatch')).toBe(false)
  })

  it('部分匹配 owner', () => {
    expect(releaseMatchesSearch(makeRelease(), 'hhelibeb')).toBe(true)
  })

  it('部分匹配 repo', () => {
    expect(releaseMatchesSearch(makeRelease(), 'relwatch')).toBe(true)
  })

  it('匹配 tag_name', () => {
    expect(releaseMatchesSearch(makeRelease(), 'v1.0.0')).toBe(true)
  })

  it('部分匹配 tag_name', () => {
    expect(releaseMatchesSearch(makeRelease(), 'v1.0')).toBe(true)
    expect(releaseMatchesSearch(makeRelease(), '1.0.0')).toBe(true)
  })

  it('匹配 release_name', () => {
    expect(releaseMatchesSearch(makeRelease(), 'RelWatch')).toBe(true)
  })

  it('匹配 body 内容（视频源：body 即简介，进 Tier1）', () => {
    const release = makeRelease({ source_type: 'bilibili', body: 'This release fixes critical bugs' })
    expect(releaseMatchesSearch(release, 'critical')).toBe(true)
    expect(releaseMatchesSearch(release, 'bugs')).toBe(true)
  })

  it('GitHub body 不进 Tier1（2 参调用不命中，需深度搜索）', () => {
    const release = makeRelease({ body: 'This release fixes critical bugs' })
    expect(releaseMatchesSearch(release, 'critical')).toBe(false)
  })

  it('body 为 null 时不报错', () => {
    expect(releaseMatchesSearch(makeRelease({ body: null }), 'critical')).toBe(false)
  })

  it('大小写不敏感', () => {
    expect(releaseMatchesSearch(makeRelease(), 'RELWATCH')).toBe(true)
    expect(releaseMatchesSearch(makeRelease(), 'HHELIBEB')).toBe(true)
    expect(releaseMatchesSearch(makeRelease(), 'V1.0.0')).toBe(true)
  })

  it('空查询返回 true', () => {
    expect(releaseMatchesSearch(makeRelease(), '')).toBe(true)
    expect(releaseMatchesSearch(makeRelease(), '   ')).toBe(true)
  })

  it('无匹配返回 false', () => {
    expect(releaseMatchesSearch(makeRelease(), 'nonexistent')).toBe(false)
    expect(releaseMatchesSearch(makeRelease(), 'v2.0.0')).toBe(false)
  })
})

// ── tokenizeQuery ────────────────────────────────────────────────

describe('tokenizeQuery', () => {
  it('按空白切词并小写', () => {
    expect(tokenizeQuery('RelWatch V1.14')).toEqual(['relwatch', 'v1.14'])
  })
  it('空查询返回空数组', () => {
    expect(tokenizeQuery('')).toEqual([])
    expect(tokenizeQuery('   ')).toEqual([])
  })
  it('版本号不按点拆分', () => {
    expect(tokenizeQuery('v1.14.0')).toEqual(['v1.14.0'])
  })
})

// ── releaseMatchesSearch — 词元 AND ──────────────────────────────

describe('releaseMatchesSearch — 词元 AND', () => {
  it('跨字段组合：relwatch v1.14', () => {
    expect(releaseMatchesSearch(makeRelease({ tag_name: 'v1.14.0' }), 'relwatch v1.14')).toBe(true)
    expect(releaseMatchesSearch(makeRelease({ tag_name: 'v1.14.0' }), 'relwatch v2.0')).toBe(false)
  })
  it('【回归】owner/repo 必须作为整体可匹配', () => {
    // 若 haystack 用 join(' ') 把 owner/repo 拆开，此用例会失败
    expect(releaseMatchesSearch(makeRelease(), 'hhelibeb/relwatch')).toBe(true)
  })
  it('中文子串匹配 ai_summary', () => {
    expect(releaseMatchesSearch(makeRelease({ ai_summary: '新增服务模式启动支持' }), '服务模式')).toBe(true)
  })
  it('【新增】视频源 body 进 Tier1（简介/标签可直接搜）', () => {
    const bili = makeRelease({ source_type: 'bilibili', body: '助眠 asmr 标签' })
    expect(releaseMatchesSearch(bili, '助眠')).toBe(true)
    expect(releaseMatchesSearch(bili, 'asmr')).toBe(true)
  })
  it('GitHub body 不进 Tier1（无论长短，一律需深度搜索）', () => {
    const gh = (b: string) => makeRelease({ source_type: 'github', body: b })
    expect(releaseMatchesSearch(gh('aaa'), 'aaa')).toBe(false)
    expect(releaseMatchesSearch(gh('b'.repeat(1000)), 'bbb')).toBe(false)
    // 显式传入 bodyFields 时可命中
    expect(releaseMatchesSearch(gh('b'.repeat(1000)), 'bbb', ['b'.repeat(1000), ''])).toBe(true)
  })
  it('【回归】Tier1 判定跟随注册表能力位（运行时覆写后自动生效）', () => {
    // 搜索分层不再镜像类型集合：把 github 临时标为 aiSummary:false，其 body 应即时进 Tier1；
    // 还原后回落 Tier2。对应 syncSourceCapabilities() 依后端 ai_eligible 覆写的场景。
    const gh = sourceTypeDefs.find(d => d.type === 'github')!
    const before = gh.aiSummary
    try {
      gh.aiSummary = false
      expect(releaseMatchesSearch(makeRelease({ source_type: 'github', body: '助眠 asmr' }), '助眠')).toBe(true)
      gh.aiSummary = true
      expect(releaseMatchesSearch(makeRelease({ source_type: 'github', body: '助眠 asmr' }), '助眠')).toBe(false)
    } finally {
      gh.aiSummary = before
    }
  })
  it('大小写不敏感', () => {
    expect(releaseMatchesSearch(makeRelease(), 'RELWATCH')).toBe(true)
  })
  it('空查询恒真', () => {
    expect(releaseMatchesSearch(makeRelease(), '')).toBe(true)
    expect(releaseMatchesSearch(makeRelease(), '   ')).toBe(true)
  })
})

// ── getSearchIndex ───────────────────────────────────────────────

describe('getSearchIndex', () => {
  it('同一数组引用只构建一次（WeakMap 幂等）', () => {
    const arr = [makeRelease()]
    expect(getSearchIndex(arr)).toBe(getSearchIndex(arr))
  })
  it('包含 owner/repo 片段', () => {
    const idx = getSearchIndex([makeRelease()])
    expect(idx[0][0]).toBe('hhelibeb/relwatch')
  })
})

// ── formatDate ────────────────────────────────────────────────────

describe('formatDate', () => {
  it('返回字符串且非空', () => {
    const result = formatDate('2025-01-15T10:30:00Z')
    expect(typeof result).toBe('string')
    expect(result.length).toBeGreaterThan(0)
  })

  it('包含日期数字', () => {
    const result = formatDate('2025-01-15T10:30:00Z')
    expect(result).toContain('2025')
    expect(result).toContain('1')
  })

  it('处理不同的日期格式', () => {
    const result1 = formatDate('2024-12-01T00:00:00Z')
    const result2 = formatDate('2024-12-31T23:59:59Z')
    expect(typeof result1).toBe('string')
    expect(typeof result2).toBe('string')
    expect(result1).not.toBe(result2)
  })

  it('空字符串返回空字符串', () => {
    expect(formatDate('')).toBe('')
  })

  it('无效日期字符串返回空字符串', () => {
    expect(formatDate('not-a-date')).toBe('')
    expect(formatDate('2024-13-01T00:00:00Z')).toBe('')
  })
})

// ── logLevelClass ─────────────────────────────────────────────────

describe('logLevelClass', () => {
  it('ERROR → log-error', () => {
    expect(logLevelClass('ERROR')).toBe('log-error')
  })

  it('WARN → log-warn', () => {
    expect(logLevelClass('WARN')).toBe('log-warn')
  })

  it('INFO → log-info', () => {
    expect(logLevelClass('INFO')).toBe('log-info')
  })

  it('DEBUG → log-info（默认分支）', () => {
    expect(logLevelClass('DEBUG')).toBe('log-info')
  })

  it('空字符串 → log-info', () => {
    expect(logLevelClass('')).toBe('log-info')
  })
})

// ── isUnreadStatus / isReadStatus ─────────────────────────────────

describe('isUnreadStatus', () => {
  it('pending → true', () => { expect(isUnreadStatus('pending')).toBe(true) })
  it('snoozed → true', () => { expect(isUnreadStatus('snoozed')).toBe(true) })
  it('clicked → false', () => { expect(isUnreadStatus('clicked')).toBe(false) })
  it('ignored → false', () => { expect(isUnreadStatus('ignored')).toBe(false) })
  it('unknown → false', () => { expect(isUnreadStatus('unknown')).toBe(false) })
  it('snoozed + future snooze_until → false', () => {
    const future = new Date(Date.now() + 60_000).toISOString()
    expect(isUnreadStatus('snoozed', future)).toBe(false)
  })
  it('snoozed + expired snooze_until → true', () => {
    const past = new Date(Date.now() - 60_000).toISOString()
    expect(isUnreadStatus('snoozed', past)).toBe(true)
  })
})

describe('isReadStatus', () => {
  it('clicked → true', () => { expect(isReadStatus('clicked')).toBe(true) })
  it('ignored → true', () => { expect(isReadStatus('ignored')).toBe(true) })
  it('pending → false', () => { expect(isReadStatus('pending')).toBe(false) })
  it('snoozed → false', () => { expect(isReadStatus('snoozed')).toBe(false) })
  it('unknown → false', () => { expect(isReadStatus('unknown')).toBe(false) })
})

// ── statusLabel ───────────────────────────────────────────────────

describe('statusLabel', () => {
  it('pending → 国际化文本', () => {
    const label = statusLabel('pending')
    // t('status.pending') — 默认 zh-CN 下应为非空字符串且与 key 不同
    expect(label).toBeTruthy()
    expect(label).not.toBe('pending')
  })

  it('snoozed → 国际化文本', () => {
    const label = statusLabel('snoozed')
    expect(label).toBeTruthy()
    expect(label).not.toBe('snoozed')
  })

  it('clicked → 国际化文本', () => {
    const label = statusLabel('clicked')
    expect(label).toBeTruthy()
    expect(label).not.toBe('clicked')
  })

  it('ignored → 国际化文本', () => {
    const label = statusLabel('ignored')
    expect(label).toBeTruthy()
    expect(label).not.toBe('ignored')
  })

  it('未知状态直接返回原值', () => {
    expect(statusLabel('unknown')).toBe('unknown')
    expect(statusLabel('processing')).toBe('processing')
  })
})

// ── statusClass ───────────────────────────────────────────────────

describe('statusClass', () => {
  it('pending → status-unread', () => { expect(statusClass('pending')).toBe('status-unread') })
  it('snoozed → status-unread', () => { expect(statusClass('snoozed')).toBe('status-unread') })
  it('snoozed + future snooze_until → status-snoozed', () => {
    const future = new Date(Date.now() + 60_000).toISOString()
    expect(statusClass('snoozed', future)).toBe('status-snoozed')
  })
  it('clicked → status-read', () => { expect(statusClass('clicked')).toBe('status-read') })
  it('ignored → status-read', () => { expect(statusClass('ignored')).toBe('status-read') })
  it('unknown → status-unknown', () => { expect(statusClass('unknown')).toBe('status-unknown') })
  it('空字符串 → status-unknown', () => { expect(statusClass('')).toBe('status-unknown') })
})

// ============ releaseMatchesSearch — source_description ============

describe('releaseMatchesSearch — source_description', () => {
  it('YouTube 频道名可被搜索命中', () => {
    const release = makeRelease({ owner: 'UCXuqSBlHAE6Xw', repo: '', source_description: '时局眼' })
    expect(releaseMatchesSearch(release, '时局眼')).toBe(true)
    expect(releaseMatchesSearch(release, '局眼')).toBe(true)
  })

  it('无 source_description 时不影响其它字段匹配', () => {
    const release = makeRelease({ owner: 'UCXuqSBlHAE6Xw', repo: '' })
    expect(releaseMatchesSearch(release, 'UCXuqSBlHAE6Xw')).toBe(true)
  })
})

// ── skillShortName ────────────────────────────────────────────────

describe('skillShortName', () => {
  it('路径指向 SKILL.md 文件时取目录名（skill 名）', () => {
    expect(skillShortName('E:\\project\\relwatch\\.pi\\skills\\commit\\SKILL.md')).toBe('commit')
    expect(skillShortName('skills/commit/SKILL.md')).toBe('commit')
    expect(skillShortName('.pi/skills/release/SKILL.md')).toBe('release')
  })

  it('纯目录路径取最后一段', () => {
    expect(skillShortName('skills/commit')).toBe('commit')
    expect(skillShortName('E:\\pi\\skills')).toBe('skills')
    expect(skillShortName('commit')).toBe('commit')
  })

  it('去掉尾部分隔符', () => {
    expect(skillShortName('skills/commit/')).toBe('commit')
    expect(skillShortName('skills\\commit\\')).toBe('commit')
  })

  it('短名含特殊字符（点/横线）不受影响', () => {
    expect(skillShortName('.pi/skills/code-review/SKILL.md')).toBe('code-review')
    expect(skillShortName('my.skill')).toBe('my.skill')
  })
})
