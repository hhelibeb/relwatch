import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import {
  AGENT_MAX_CONCURRENCY_LIMIT,
  AGENT_TIMEOUT_MAX,
  AGENT_TIMEOUT_MIN,
} from '../api/agent'
import { MAIN_MIN_HEIGHT, MAIN_MIN_WIDTH } from '../composables/useAgentPanelLayout'

const root = process.cwd()

function readProjectFile(relPath: string): string {
  return readFileSync(resolve(root, relPath), 'utf-8')
}

function extractNumber(source: string, pattern: RegExp, label: string): number {
  const m = source.match(pattern)
  expect(m, `未找到常量 ${label}`).not.toBeNull()
  return Number(m![1])
}

describe('跨端手工同步常量静态对拍', () => {
  it('主窗口最小尺寸：useAgentPanelLayout.ts ↔ src-tauri/src/lib.rs', () => {
    const be = readProjectFile('src-tauri/src/lib.rs')
    const m = be.match(/\.min_inner_size\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*\)/)
    expect(m, 'lib.rs 中应声明 .min_inner_size(...)').not.toBeNull()

    expect(MAIN_MIN_WIDTH).toBe(Number(m![1]))
    expect(MAIN_MIN_HEIGHT).toBe(Number(m![2]))
  })

  it('界面缩放范围：useFontScale.ts ↔ src-tauri/src/db/settings.rs', () => {
    const fe = readProjectFile('src/composables/useFontScale.ts')
    const be = readProjectFile('src-tauri/src/db/settings.rs')

    expect(extractNumber(fe, /const SCALE_MIN\s*=\s*(\d+)/, 'SCALE_MIN')).toBe(
      extractNumber(be, /const FONT_SCALE_MIN:\s*i64\s*=\s*(\d+)/, 'FONT_SCALE_MIN'),
    )
    expect(extractNumber(fe, /const SCALE_MAX\s*=\s*(\d+)/, 'SCALE_MAX')).toBe(
      extractNumber(be, /const FONT_SCALE_MAX:\s*i64\s*=\s*(\d+)/, 'FONT_SCALE_MAX'),
    )
  })

  it('Agent 并发与超时边界：src/api/agent.ts ↔ src-tauri/src/db/agent.rs', () => {
    const be = readProjectFile('src-tauri/src/db/agent.rs')

    expect(AGENT_MAX_CONCURRENCY_LIMIT).toBe(
      extractNumber(be, /const MAX_AGENT_CONCURRENCY:\s*i64\s*=\s*(\d+)/, 'MAX_AGENT_CONCURRENCY'),
    )
    expect(AGENT_TIMEOUT_MIN).toBe(
      extractNumber(be, /const MIN_AGENT_TIMEOUT_SECONDS:\s*i64\s*=\s*(\d+)/, 'MIN_AGENT_TIMEOUT_SECONDS'),
    )
    expect(AGENT_TIMEOUT_MAX).toBe(
      extractNumber(be, /const MAX_AGENT_TIMEOUT_SECONDS:\s*i64\s*=\s*(\d+)/, 'MAX_AGENT_TIMEOUT_SECONDS'),
    )
  })
})
