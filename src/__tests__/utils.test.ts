import { describe, it, expect, vi } from 'vitest'
import {
  importanceLabel,
  formatDate,
  releaseMatchesSearch,
  logLevelClass,
  statusLabel,
  statusClass,
  isUnreadStatus,
  isReadStatus,
} from '../utils'

// ── releaseMatchesSearch ──────────────────────────────────────────

function makeRelease(overrides = {}) {
  return {
    owner: 'hhelibeb',
    repo: 'relwatch',
    tag_name: 'v1.0.0',
    release_name: 'RelWatch 1.0.0',
    body: null,
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

  it('匹配 body 内容', () => {
    const release = makeRelease({ body: 'This release fixes critical bugs' })
    expect(releaseMatchesSearch(release, 'critical')).toBe(true)
    expect(releaseMatchesSearch(release, 'bugs')).toBe(true)
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

// ── importanceLabel ───────────────────────────────────────────────

describe('importanceLabel', () => {
  it('返回空字符串当输入为 null', () => {
    expect(importanceLabel(null)).toBe('')
  })

  it('返回空字符串当输入为空字符串', () => {
    expect(importanceLabel('')).toBe('')
  })

  it('格式化"大"', () => {
    expect(importanceLabel('大')).toBe('重要度: 🔴 大')
  })

  it('格式化"中"', () => {
    expect(importanceLabel('中')).toBe('重要度: 🟡 中')
  })

  it('格式化"小"', () => {
    expect(importanceLabel('小')).toBe('重要度: 🟢 小')
  })

  it('未知值直接返回', () => {
    expect(importanceLabel('未知')).toBe('未知')
    expect(importanceLabel('critical')).toBe('critical')
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
  it('clicked → status-read', () => { expect(statusClass('clicked')).toBe('status-read') })
  it('ignored → status-read', () => { expect(statusClass('ignored')).toBe('status-read') })
  it('unknown → status-unknown', () => { expect(statusClass('unknown')).toBe('status-unknown') })
  it('空字符串 → status-unknown', () => { expect(statusClass('')).toBe('status-unknown') })
})
