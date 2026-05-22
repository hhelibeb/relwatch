import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: vi.fn(),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(() => ({
    onCloseRequested: vi.fn(),
    hide: vi.fn(),
    show: vi.fn(),
    setFocus: vi.fn(),
    isVisible: vi.fn(() => true),
  })),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(),
}))
vi.mock('../i18n', () => ({
  t: vi.fn((key: string) => key),
}))

import { parseGitHubUrl } from '../api/sources'
import { translateError } from '../api/client'

describe('parseGitHubUrl', () => {
  it('parses standard GitHub URL', () => {
    const result = parseGitHubUrl('https://github.com/microsoft/vscode')
    expect(result).toEqual({ owner: 'microsoft', repo: 'vscode' })
  })

  it('parses URL with trailing slash', () => {
    const result = parseGitHubUrl('https://github.com/tauri-apps/tauri/')
    expect(result).toEqual({ owner: 'tauri-apps', repo: 'tauri' })
  })

  it('parses URL with query string and fragment', () => {
    const result = parseGitHubUrl('https://github.com/rust-lang/rust?tab=readme')
    expect(result).toEqual({ owner: 'rust-lang', repo: 'rust' })
  })

  it('returns null for non-GitHub URL', () => {
    const result = parseGitHubUrl('https://gitlab.com/user/repo')
    expect(result).toBeNull()
  })

  it('returns null for incomplete URL', () => {
    const result = parseGitHubUrl('github.com/user')
    expect(result).toBeNull()
  })

  it('returns null for empty string', () => {
    const result = parseGitHubUrl('')
    expect(result).toBeNull()
  })

  it('returns null for just github.com', () => {
    const result = parseGitHubUrl('https://github.com/user/')
    expect(result).toBeNull()
  })

  it('parses URL with www subdomain and dot in repo name', () => {
    const result = parseGitHubUrl('https://www.github.com/vercel/next.js')
    expect(result).toEqual({ owner: 'vercel', repo: 'next.js' })
  })

  it('parses owner/repo format', () => {
    const result = parseGitHubUrl('microsoft/vscode')
    expect(result).toEqual({ owner: 'microsoft', repo: 'vscode' })
  })

  it('parses owner/repo with hyphens', () => {
    const result = parseGitHubUrl('my-org/my-repo')
    expect(result).toEqual({ owner: 'my-org', repo: 'my-repo' })
  })

  it('parses owner/repo with dots and underscores', () => {
    const result = parseGitHubUrl('user.name/repo_test')
    expect(result).toEqual({ owner: 'user.name', repo: 'repo_test' })
  })

  it('rejects owner/repo with spaces', () => {
    const result = parseGitHubUrl('user /repo')
    expect(result).toBeNull()
  })

  it('rejects owner/repo without slash', () => {
    const result = parseGitHubUrl('justmonorepo')
    expect(result).toBeNull()
  })

  it('rejects owner starting with hyphen', () => {
    const result = parseGitHubUrl('-badowner/repo')
    expect(result).toBeNull()
  })
})

// ── translateError ────────────────────────────────────────────────

describe('translateError', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('移除 "Error: " 前缀并调用 t()', () => {
    // 默认 mock 的 t 返回 key 本身
    expect(translateError('Error: err.source.not_found')).toBe('err.source.not_found')
  })

  it('不以 "err." 开头时返回原始字符串（含 Error: 前缀），不调用 t()', async () => {
    const { t } = await import('../i18n')
    // translateError 在非 err. 前缀时返回 raw（含 Error:），而非 msg
    expect(translateError('Error: something went wrong')).toBe('Error: something went wrong')
    expect(t).not.toHaveBeenCalled()
  })

  it('无 "Error: " 前缀的字符串直接返回', async () => {
    const { t } = await import('../i18n')
    expect(translateError('normal message')).toBe('normal message')
    expect(t).not.toHaveBeenCalled()
  })

  it('仅 "Error: " 前缀无内容返回原字符串', () => {
    // 同上：非 err. 前缀返回 raw
    expect(translateError('Error: ')).toBe('Error: ')
  })

  it('带 pipe 分隔的参数传递给 t()', async () => {
    const { t } = await import('../i18n')
    translateError('Error: err.add_failed|microsoft/vscode')
    expect(t).toHaveBeenCalledWith('err.add_failed', 'microsoft/vscode')
  })

  it('带多个 pipe 分隔的参数', async () => {
    const { t } = await import('../i18n')
    translateError('Error: err.format|{0}|{1}|{2}')
    expect(t).toHaveBeenCalledWith('err.format', '{0}', '{1}', '{2}')
  })
})

// ── invokeI18n（通过 listSources 间接测试错误处理） ─────────────

describe('invokeI18n 错误处理', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('invoke 成功时返回结果', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue([{ id: 1, owner: 'test', repo: 'repo' }])

    const { listSources } = await import('../api/sources')
    const result = await listSources()
    expect(result).toEqual([{ id: 1, owner: 'test', repo: 'repo' }])
    expect(invoke).toHaveBeenCalledWith('list_sources', undefined)
  })

  it('invoke 抛出 Tauri 错误时通过 translateError 转换', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Error: err.db_query_failed'))

    const { listSources } = await import('../api/sources')
    await expect(listSources()).rejects.toThrow('err.db_query_failed')
  })

  it('invoke 抛出非 Error 类型时转换为 Error', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockRejectedValue('string error')

    const { listSources } = await import('../api/sources')
    await expect(listSources()).rejects.toThrow('string error')
  })

  it('invoke 抛出无 "err." 前缀的错误时不调用 t()', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    const { t } = await import('../i18n')
    ;(invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Error: network timeout'))

    const { listSources } = await import('../api/sources')
    await expect(listSources()).rejects.toThrow('network timeout')
    expect(t).not.toHaveBeenCalled()
  })

  it('invoke 带参数调用', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    ;(invoke as ReturnType<typeof vi.fn>).mockResolvedValue([])

    const { getLogs } = await import('../api/logs')
    await getLogs(50)
    expect(invoke).toHaveBeenCalledWith('get_logs', { limit: 50 })
  })
})
