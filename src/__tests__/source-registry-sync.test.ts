/**
 * 前后端源类型注册表对拍测试（防漂移兜底）。
 *
 * 前端 `sourceTypeDefs`（src/api/source-registry.ts）与后端 `ADAPTERS`
 * （src-tauri/src/source.rs）是两份独立清单。本测试直接读取 Rust 源码文本，
 * 静态提取后端注册表内容，与前端注册表逐项对比：
 * - 类型集合必须一致（新增源漏登记任一侧即失败）；
 * - aiSummary=false 与后端 ai_eligible=false 覆写一一对应。
 *
 * 能力位的权威出口是后端只读命令 list_source_types（由 ADAPTERS 动态枚举），
 * 前端如接入该命令则本测试可作为补充防线。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { sourceTypeDefs } from '../api/source-registry'

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

  it('aiSummary=false 与后端 ai_eligible=false 覆写一一对应', () => {
    const frontendDisabled = sourceTypeDefs
      .filter(d => d.aiSummary === false)
      .map(d => d.type)
      .sort()
    // 后端覆写 ai_eligible=false 的适配器：以独立文件方式枚举（文件即适配器）。
    // 若新增 adapter 文件，请同时加入此数组，并保持与 ADAPTERS 登记一致。
    const backendDisabled = ['youtube', 'bilibili']
      .filter(t => /fn ai_eligible\(&self\) -> bool\s*\{\s*false\s*\}/.test(readRust(`${t}.rs`)))
      .sort()
    expect(backendDisabled).toEqual(frontendDisabled)
  })
})
