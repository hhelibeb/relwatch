#!/usr/bin/env node
// CHANGELOG 长度自检 —— release skill Step 4.1.1
// Usage: node scripts/check-changelog-length.mjs [版本号]
//   不传版本号时检查 [Unreleased] 区块
// 退出码：0 通过 / 1 超限
import fs from 'node:fs'

const MAX_ITEM = 100 // 单条字符上限
const MAX_TOTAL = 800 // 单版总字数上限

const version = process.argv[2] ?? 'Unreleased'
const text = fs.readFileSync('CHANGELOG.md', 'utf8')

// 定位区块：从 "## [<ver>]" 到下一个 "## ["
const lines = text.split('\n')
const start = lines.findIndex((l) => l.startsWith(`## [${version}]`))
if (start === -1) {
  console.error(`未找到 CHANGELOG 区块: ## [${version}]`)
  process.exit(1)
}
let end = lines.length
for (let i = start + 1; i < lines.length; i++) {
  if (lines[i].startsWith('## [')) {
    end = i
    break
  }
}

const items = lines
  .slice(start + 1, end)
  .filter((l) => l.trim().startsWith('- '))
  .map((l) => l.trim())

const chars = items.reduce((a, l) => a + l.length, 0)
const over = items.filter((l) => l.length > MAX_ITEM)

console.log(`区块: [${version}]`)
console.log(`条数: ${items.length} | 总字数: ${chars} / ${MAX_TOTAL} | 单条上限: ${MAX_ITEM}`)

const problems = []
if (chars > MAX_TOTAL) problems.push(`总字数超限 ${chars} > ${MAX_TOTAL}`)
if (over.length) {
  problems.push(`超长条目 ${over.length} 条:`)
  over.forEach((l) => problems.push(`  ${l.length} 字: ${l.slice(0, 40)}…`))
}

if (problems.length) {
  console.error('\n❌ 未通过长度检查:')
  problems.forEach((p) => console.error('  ' + p))
  console.error('\n按 Step 4.1.1 压缩后再进入 Step 4.3（不写实现细节/根因/影响面/测试明细）。')
  process.exit(1)
}

console.log('\n✅ 长度检查通过')
