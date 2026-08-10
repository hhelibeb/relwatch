import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'

/**
 * i18n 键一致性测试：
 * 1. zh-CN 与 en-US 字典键集合必须完全相等（防漏译/多余键）；
 * 2. Rust 命令层使用的每个 `err.*` key 都必须在前端两个字典中有翻译
 *    （否则错误会以裸 key 形式显示给用户）。
 */

const I18N_DIR = path.resolve(process.cwd(), 'src/i18n')

function extractKeys(file: string): string[] {
  const src = fs.readFileSync(file, 'utf-8')
  return [...src.matchAll(/^\s*'([^']+)':/gm)].map((m) => m[1])
}

/** 递归收集 src-tauri/src 下所有 .rs 文件中出现的 `"err.xxx` 引用 */
function rustErrKeys(): string[] {
  const root = path.resolve(process.cwd(), 'src-tauri/src')
  const keys = new Set<string>()
  const walk = (dir: string): void => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith('.rs')) {
        const src = fs.readFileSync(full, 'utf-8')
        for (const m of src.matchAll(/"err\.[a-z_0-9.]+/g)) keys.add(m[0].slice(1))
      }
    }
  }
  walk(root)
  return [...keys].sort()
}

describe('i18n 键一致性', () => {
  const zhKeys = extractKeys(path.join(I18N_DIR, 'zh-CN.ts'))
  const enKeys = extractKeys(path.join(I18N_DIR, 'en-US.ts'))

  it('zh-CN 与 en-US 键集合完全相等', () => {
    expect(zhKeys.length).toBeGreaterThan(0)
    expect(enKeys.length).toBeGreaterThan(0)
    expect(zhKeys.sort()).toEqual(enKeys.sort())
  })

  it('Rust 侧所有 err.* key 在 zh-CN 与 en-US 中都有翻译', () => {
    const zhSet = new Set(zhKeys)
    const enSet = new Set(enKeys)
    const missingZh = rustErrKeys().filter((k) => !zhSet.has(k))
    const missingEn = rustErrKeys().filter((k) => !enSet.has(k))
    expect(missingZh, `zh-CN 缺失翻译: ${missingZh.join(', ')}`).toEqual([])
    expect(missingEn, `en-US 缺失翻译: ${missingEn.join(', ')}`).toEqual([])
  })
})
