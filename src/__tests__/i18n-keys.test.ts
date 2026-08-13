import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import { sourceTypeDefs } from '../api/source-registry'

/**
 * i18n 键一致性测试：
 * 1. zh-CN 与 en-US 字典键集合必须完全相等（防漏译/多余键）；
 * 2. Rust 命令层使用的每个 `err.*` key 都必须在前端两个字典中有翻译
 *    （否则错误会以裸 key 形式显示给用户）；
 * 3. sourceTypeDefs 的每个 titleKey 都必须在两个字典中有翻译。
 */

const I18N_DIR = path.resolve(process.cwd(), 'src/i18n')

function extractKeys(file: string): string[] {
  const src = fs.readFileSync(file, 'utf-8')
  return [...src.matchAll(/^\s*'([^']+)':/gm)].map((m) => m[1])
}

/** 递归收集 src-tauri/src 下生产代码中出现的 `"err.xxx` 引用（跳过测试块） */
function rustErrKeys(): string[] {
  const root = path.resolve(process.cwd(), 'src-tauri/src')
  const keys = new Set<string>()
  const walk = (dir: string): void => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith('.rs')) {
        const src = fs.readFileSync(full, 'utf-8')
        const prod = stripTestBlocks(src)
        for (const m of prod.matchAll(/"err\.[a-z_0-9.]+/g)) keys.add(m[0].slice(1))
      }
    }
  }
  walk(root)
  return [...keys].sort()
}

/**
 * 剥离 #[cfg(test)] / mod tests 块（花括号配对），只保留生产代码。
 * 测试代码中的 write_log_key 使用测试键（如 test.message），不应参与存在性校验。
 */
function stripTestBlocks(src: string): string {
  const re = /^\s*(?:#\[cfg\(test\)\]|mod tests\s*\{)/gm
  let result = ''
  let last = 0
  let m: RegExpExecArray | null
  while ((m = re.exec(src))) {
    result += src.slice(last, m.index)
    const open = src.indexOf('{', m.index)
    if (open === -1) break
    let depth = 0
    let i = open
    for (; i < src.length; i++) {
      const c = src[i]
      if (c === '{') depth++
      else if (c === '}') {
        depth--
        if (depth === 0) break
      }
    }
    last = i + 1
    re.lastIndex = last
  }
  result += src.slice(last)
  return result
}

/** 递归收集 src-tauri/src 下生产代码中 write_log_key 调用使用的日志 key（跳过测试块） */
function rustLogKeys(): string[] {
  const root = path.resolve(process.cwd(), 'src-tauri/src')
  const keys = new Set<string>()
  const walk = (dir: string): void => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith('.rs')) {
        const src = fs.readFileSync(full, 'utf-8')
        const prod = stripTestBlocks(src)
        // write_log_key(conn, level, "key", args) —— 支持跨行调用；
        // 第一参数限定为标识符（&conn / conn / &tx），避免误匹配函数定义本身
        const re = /write_log_key\s*\(\s*&?\w+\s*,\s*"(?:INFO|WARN|ERROR|DEBUG)"\s*,\s*"([^"]+)"/g
        for (const m of prod.matchAll(re)) keys.add(m[1])
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

  it('Rust 侧所有 write_log_key 调用键在 zh-CN 与 en-US 中都有翻译', () => {
    const zhSet = new Set(zhKeys)
    const enSet = new Set(enKeys)
    const missingZh = rustLogKeys().filter((k) => !zhSet.has(k))
    const missingEn = rustLogKeys().filter((k) => !enSet.has(k))
    expect(missingZh, `zh-CN 缺失日志键: ${missingZh.join(', ')}`).toEqual([])
    expect(missingEn, `en-US 缺失日志键: ${missingEn.join(', ')}`).toEqual([])
  })

  it('sourceTypeDefs 的每个 titleKey 在 zh-CN 与 en-US 中都有翻译', () => {
    const zhSet = new Set(zhKeys)
    const enSet = new Set(enKeys)
    const missing = sourceTypeDefs
      .filter((d) => !zhSet.has(d.titleKey) || !enSet.has(d.titleKey))
      .map((d) => d.titleKey)
    expect(missing, `缺失 source.type_* 翻译: ${missing.join(', ')}`).toEqual([])
  })
})
