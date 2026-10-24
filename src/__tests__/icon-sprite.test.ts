import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

/**
 * 图标 sprite 引用守卫。
 *
 * 背景：AgentComposer 的实体菜单分组图标写了 `/icons.svg#source-icon`，而
 * sprite 里只有 `sources-icon`。少了这个 s，`<use>` 指向不存在的 symbol，
 * 渲染结果是空白——不报错、不抛异常，jsdom 与 DOM 断言都不会失败，
 * 只有人肉看一眼才发现。故拿 sprite 的 id 表与全部引用做对账。
 */

const ROOT = process.cwd()
const SRC = resolve(ROOT, 'src')
const SPRITE = readFileSync(resolve(ROOT, 'public', 'icons.svg'), 'utf8')

function spriteIds(): Set<string> {
  return new Set([...SPRITE.matchAll(/<symbol[^>]*\bid="([^"]+)"/g)].map((m) => m[1]))
}

/** 引用源：src 下除 __tests__（夹具里会写占位 id）以外的全部 .vue/.ts */
function referenceSources(): Array<{ file: string; text: string }> {
  const out: Array<{ file: string; text: string }> = []
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === '__tests__') continue
      const full = join(dir, entry.name)
      if (entry.isDirectory()) {
        walk(full)
        continue
      }
      if (!entry.name.endsWith('.vue') && !entry.name.endsWith('.ts')) continue
      out.push({ file: full.slice(SRC.length + 1).replace(/\\/g, '/'), text: readFileSync(full, 'utf8') })
    }
  }
  walk(SRC)
  return out
}

describe('图标 sprite 守卫', () => {
  it('sprite 里确实有 symbol', () => {
    expect(spriteIds().size).toBeGreaterThan(0)
  })

  it('所有 /icons.svg#<id> 引用都能在 sprite 中找到', () => {
    const ids = spriteIds()
    const offenders = referenceSources().flatMap(({ file, text }) =>
      [...text.matchAll(/\/icons\.svg#([A-Za-z0-9_-]+)/g)]
        .map((m) => m[1])
        .filter((id) => !ids.has(id))
        .map((id) => `${file}: #${id}`),
    )
    expect(offenders).toEqual([])
  })
})
