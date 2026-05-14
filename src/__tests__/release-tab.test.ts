import { describe, expect, it } from 'vitest'
import { releaseMatchesSearch } from '../utils'

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

describe('ReleaseTab search', () => {
  it('matches the full owner/repo name', () => {
    expect(releaseMatchesSearch(makeRelease(), 'hhelibeb/relwatch')).toBe(true)
    expect(releaseMatchesSearch(makeRelease({ owner: 'microsoft', repo: 'vscode' }), 'hhelibeb/relwatch')).toBe(false)
  })
})
