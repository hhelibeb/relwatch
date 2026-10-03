#!/usr/bin/env node
// 版本号一致性自检 —— CI 门禁 + release skill Step 7
// Usage: node scripts/check-version.mjs
//
// 为什么必须有这道门：`package.json` 是 private 包不发布，版本号写错没有编译器会报错；
// 而 `src-tauri/tauri.conf.json` 是 tauri-action 解析 `__VERSION__` 的唯一来源
// （tagName / releaseName / latest.json 全由它填充，只有它缺失或指向 .json 时才回退
// Cargo.toml）。漏改它 → 推 tag vX 却把产物发到上一个版本的 release 上 → 用户端 updater
// 拿到 <= 当前版本 → 「已是最新」，静默不更新；Release 页面还一切正常。
import fs from 'node:fs'

const readJsonVersion = (path) => JSON.parse(fs.readFileSync(path, 'utf8')).version
// [package] 段的 version 是行首匹配；依赖行里的 version 都嵌在表内联写法里，不会命中
const cargoVersion = fs
  .readFileSync('src-tauri/Cargo.toml', 'utf8')
  .match(/^version\s*=\s*"([^"]+)"/m)?.[1]

const versions = {
  'package.json': readJsonVersion('package.json'),
  'src-tauri/tauri.conf.json': readJsonVersion('src-tauri/tauri.conf.json'),
  'src-tauri/Cargo.toml': cargoVersion,
}

if (new Set(Object.values(versions)).size !== 1) {
  for (const [file, version] of Object.entries(versions)) {
    console.error(`  ${file}: ${version ?? '(未找到 version)'}`)
  }
  console.error('❌ 版本号不一致：三处必须相同（tauri.conf.json 决定发布产物版本）')
  process.exit(1)
}

console.log(`✅ 版本号一致：${versions['package.json']}`)
