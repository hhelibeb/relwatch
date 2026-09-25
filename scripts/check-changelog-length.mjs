#!/usr/bin/env node
// CHANGELOG 详略自检 —— release skill Step 4.1.1
// Usage: node scripts/check-changelog-length.mjs [版本号]
//   不传版本号时检查 [Unreleased] 区块
//
// 设计原则（重要）：控制的是「力度」不是「总量」。
//   - 条数不受限：改了多少点就该有多少条
//   - 总量不受限：不设单版总字数上限
//   - 只对「明显在讲实现」的超长条目报警，供人工判断
//   - 安全条目（含 RUSTSEC / CVE / GHSA 编号）不报警
// 退出码恒为 0 —— 这是辅助视线，不是裁判，不阻断流程。
import fs from 'node:fs'

const WARN_ITEM = 150 // 超过此长度才提示「可能在讲实现」

const version = process.argv[2] ?? 'Unreleased'
const text = fs.readFileSync('CHANGELOG.md', 'utf8')

// 定位区块：从 "## [<ver>]" 到下一个 "## ["
const lines = text.split('\n')
const start = lines.findIndex((l) => l.startsWith(`## [${version}]`))
if (start === -1) {
  console.error(`未找到 CHANGELOG 区块: ## [${version}]`)
  process.exit(0)
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
const avg = items.length ? Math.round(chars / items.length) : 0

console.log(`区块: [${version}]`)
console.log(`条数: ${items.length}（不受限）`)
console.log(`总字数: ${chars}（不受限，仅供参考）| 均长: ${avg}`)
console.log('')

// 安全条目含 advisory 编号 → 天然写得长，不报警
const isSecurity = (l) => /RUSTSEC|CVE|GHSA/i.test(l)
const suspects = items.filter((l) => l.length > WARN_ITEM && !isSecurity(l))

items.forEach((l) => {
  const flag = l.length > WARN_ITEM ? (isSecurity(l) ? '  ℹ️' : '  ⚠️') : '   '
  console.log(`${flag} ${String(l.length).padStart(4)} 字  ${l.slice(0, 46)}…`)
})

if (suspects.length) {
  console.log(`\n⚠️  ${suspects.length} 条超过 ${WARN_ITEM} 字，可能在讲实现（非硬性）：`)
  suspects.forEach((l) =>
    console.log(`   ${l.length} 字: ${l.slice(0, 50)}…`),
  )
  console.log(
    '\n逐条问自己：删掉这段，用户还能判断「要不要升级」吗？能就删。' +
      '\n若确认写的是结论（而非函数名/根因/影响面/测试明细），放行即可。',
  )
} else {
  console.log('\n✅ 无明显讲实现的超长条目')
}
