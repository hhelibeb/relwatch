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

import { parseGitHubUrl } from '../api'

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
})
