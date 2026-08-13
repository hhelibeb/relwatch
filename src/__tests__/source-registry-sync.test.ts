/**
 * 前后端源类型注册表对拍测试（防漂移兜底）。
 *
 * 前端 `sourceTypeDefs`（src/api/source-registry.ts）与后端 `ADAPTERS`
 * （src-tauri/src/source.rs）是两份独立清单。本测试直接读取 Rust 源码文本，
 * 静态提取后端注册表内容，与前端注册表对比类型集合（新增源漏登记任一侧即失败）。
 *
 * 能力位不再静态对拍：aiSummary 由运行时同步 `syncSourceCapabilities` 从后端
 * 只读命令 `list_source_types`（ADAPTERS 动态枚举）下发，实现即事实。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { sourceTypeDefs, getSourceTypeDef, sourceDisplayName } from '../api/source-registry'
import type { Source } from '../api/sources'

// vitest 工作目录为仓库根，Rust 源码在 src-tauri/src/
const rustDir = resolve(process.cwd(), 'src-tauri/src')

function readRust(name: string): string {
  return readFileSync(resolve(rustDir, name), 'utf-8')
}

/** 提取 ADAPTERS 注册表里的 ("xxx", || 类型字符串列表。 */
function extractAdapterTypes(): string[] {
  const rust = readRust('source.rs')
  const m = rust.match(/ADAPTERS\.get_or_init\(\|\| \{\s*vec!\[([\s\S]*?)\]\s*\}\)/)
  expect(m, 'source.rs 中应能找到 ADAPTERS 注册表（若重构了注册表写法，请同步本测试）').not.toBeNull()
  const types = [...(m![1].matchAll(/\("(\w+)", \|\|/g) ?? [])].map(x => x[1])
  expect(types.length).toBeGreaterThan(0)
  return types
}

describe('前端 sourceTypeDefs 与后端 ADAPTERS 对拍', () => {
  it('类型集合一致：新增监控源必须同时登记后端 ADAPTERS 与前端 sourceTypeDefs', () => {
    const backend = extractAdapterTypes().sort()
    const frontend = sourceTypeDefs.map(d => d.type).sort()
    expect(frontend).toEqual(backend)
  })

  it('未知类型优雅降级：getSourceTypeDef 返回 undefined，展示回退 owner/repo', () => {
    // 前端静态表只登记已知类型；后端新增类型未登记时 UI 不应崩溃
    expect(getSourceTypeDef('gitlab')).toBeUndefined()
    const src = { source_type: 'gitlab', owner: 'foo', repo: 'bar' } as unknown as Source
    expect(sourceDisplayName(src)).toBe('foo/bar')
  })
})
