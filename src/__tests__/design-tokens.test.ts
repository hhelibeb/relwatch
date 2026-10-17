import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

/**
 * 设计 token 守卫。
 *
 * 背景：Agent 面板曾并行维护第二套色板（50 处硬编码色，含 10 个色值），
 * 既不对应 --primary 也不随深浅主题切换；另有 --accent / --mono-font /
 * --app-padding-y / --bg-input 四个变量「引用有、定义无」，全靠 fallback 存活。
 * 这类漂移只发生在 CSS 层，DOM 断言（agent-* 测试）抓不到，故用源码扫描兜住。
 */

const SRC = resolve(process.cwd(), 'src')
const AGENT_DIR = join(SRC, 'components', 'agent')
const STYLE_CSS = stripComments(readFileSync(join(SRC, 'style.css'), 'utf8'))

/** 已被消灭的旧色板：任一回归即说明又出现了第二套品牌色 */
const LEGACY_PALETTE = [
  '#2e6fd0', // 旧品牌蓝（与 --primary #3e63dd 并行的第二套）
  '#2e9e5b',
  '#d64545',
  '#c0392b',
  '#d08a2e',
  '#b0882e',
  '#8a6d3b',
  '#8a6d1f',
  '#35a06b',
  '#8a8a8a',
]

/** 面板依赖的成对 token：必须在 :root 与 [data-theme="dark"] 各定义一次 */
const THEMED_TOKENS = [
  '--primary',
  '--primary-hover',
  '--primary-soft-bg',
  '--primary-soft-border',
  '--primary-soft-text',
  '--success',
  '--warning',
  '--danger',
  '--danger-hover',
  '--warning-soft-bg',
  '--warning-soft-text',
  '--danger-soft-bg',
  '--text-muted',
]

/** 曾「引用有定义无」的变量：必须真的定义，且引用处不再藏 fallback */
const BACKFILLED_TOKENS = ['--accent', '--mono-font', '--app-padding-y']

function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, '')
}

/** 取 .css 全文；.vue 只取全部 style 块，模板与脚本里的文案不参与判定 */
function agentStyleSources(): Array<{ file: string; css: string }> {
  const files = readdirSync(AGENT_DIR)
    .filter((f) => f.endsWith('.vue') || f.endsWith('.css'))
    .map((f) => join(AGENT_DIR, f))
  // 面板编排层不在 agent/ 目录，但同属这套色板
  files.push(join(SRC, 'components', 'AgentWorkspace.vue'))

  return files.map((full) => {
    const file = full.split(/[\\/]/).pop() as string
    const text = readFileSync(full, 'utf8')
    const raw = file.endsWith('.css')
      ? text
      : [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n')
    return { file, css: stripComments(raw) }
  })
}

/** 取某个选择器块内声明的全部自定义属性名 */
function declarationsIn(selector: string): string[] {
  const start = STYLE_CSS.indexOf(selector)
  if (start === -1) return []
  const open = STYLE_CSS.indexOf('{', start)
  const close = STYLE_CSS.indexOf('}', open)
  return [...STYLE_CSS.slice(open, close).matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1])
}

/** 全量样式源：尺度守卫要覆盖每个组件，不限于 Agent 面板 */
let allSourcesCache: Array<{ file: string; css: string }> | null = null
function allStyleSources(): Array<{ file: string; css: string }> {
  if (allSourcesCache) return allSourcesCache
  const sources = [{ file: 'src/style.css', css: STYLE_CSS }]
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) {
        walk(full)
        continue
      }
      if (!entry.name.endsWith('.css') && !entry.name.endsWith('.vue')) continue
      const text = readFileSync(full, 'utf8')
      const css = entry.name.endsWith('.css')
        ? text
        : [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n')
      sources.push({ file: full.slice(SRC.length + 1), css: stripComments(css) })
    }
  }
  walk(SRC)
  allSourcesCache = sources
  return sources
}

/** 尺度：字号 6 档 / 圆角 5 档 / 层级 7 档 / 动效 3 时长 + 1 缓动 */
const SCALE_TOKENS = [
  '--fs-caption',
  '--fs-meta',
  '--fs-body',
  '--fs-title',
  '--fs-display',
  '--fs-metric',
  '--radius-xs',
  '--radius-sm',
  '--radius',
  '--radius-lg',
  '--radius-pill',
  '--z-sticky',
  '--z-menu',
  '--z-popover',
  '--z-toast',
  '--z-modal',
  '--z-popover-modal',
  '--z-dev',
  '--ease-out',
  '--dur-hover',
  '--dur-state',
  '--dur-overlay',
]

describe('设计 token 守卫', () => {
  it('Agent 面板不含裸 hex（品牌色只能经 token 取用）', () => {
    const offenders = agentStyleSources().flatMap(({ file, css }) =>
      [...css.matchAll(/#[0-9a-fA-F]{3,8}\b/g)].map((m) => `${file}: ${m[0]}`),
    )
    expect(offenders).toEqual([])
  })

  it('旧色板色值不再出现', () => {
    const css = agentStyleSources()
      .map((s) => s.css)
      .join('\n')
    expect(LEGACY_PALETTE.filter((c) => css.includes(c))).toEqual([])
  })

  it('语义色在深浅两套主题下成对定义', () => {
    const light = declarationsIn(':root {')
    const dark = declarationsIn('[data-theme="dark"] {')
    expect(light.length).toBeGreaterThan(0)
    expect(dark.length).toBeGreaterThan(0)
    expect(THEMED_TOKENS.filter((t) => !light.includes(t))).toEqual([])
    expect(THEMED_TOKENS.filter((t) => !dark.includes(t))).toEqual([])
  })

  it('补齐的变量已定义，且引用处不再藏 fallback 色值', () => {
    expect(BACKFILLED_TOKENS.filter((t) => !declarationsIn(':root {').includes(t))).toEqual([])

    const styleSources = allStyleSources()

    const offenders = styleSources.flatMap(({ file, css }) =>
      BACKFILLED_TOKENS.filter((token) => new RegExp(`var\\(\\s*${token}\\s*,`).test(css)).map(
        (token) => `${file}: var(${token}, …)`,
      ),
    )
    expect(offenders).toEqual([])
  })
})

/**
 * 尺度守卫。
 *
 * 背景：界面曾并行用着 9 档字号（9–20px）、8 种裸圆角、14 档 z-index、7 种过渡时长。
 * 层级实际由颜色承担，浮层顺序靠「数字比大小」的口头约定。收口成 token 之后，
 * 组件里再写裸值就等于重新开一条旁路，故用同一套源码扫描钉住。
 */
describe('尺度守卫', () => {
  it('尺度 token 已定义', () => {
    const defined = declarationsIn(':root {')
    expect(SCALE_TOKENS.filter((t) => !defined.includes(t))).toEqual([])
  })

  it('字号不出现裸 px', () => {
    const offenders = allStyleSources().flatMap(({ file, css }) =>
      [...css.matchAll(/font-size:\s*\d+(?:\.\d+)?px/g)].map((m) => `${file}: ${m[0]}`),
    )
    expect(offenders).toEqual([])
  })

  // Markdown 标题必须相对正文缩放（引用块、嵌套内容里才不会跳档），这一处保留 em。
  // 其余地方写 em/rem/% 与写裸 px 等价：都能绕开 6 档尺度，只在 MarkdownContent 放行。
  it('相对字号只出现在 Markdown 的相对排版里', () => {
    // file 用平台路径分隔符拼出，这里只认文件名
    const isMarkdown = (file: string) => /MarkdownContent\.vue$/.test(file)
    const offenders = allStyleSources()
      .filter(({ file }) => !isMarkdown(file))
      .flatMap(({ file, css }) =>
        [...css.matchAll(/font-size:\s*[\d.]+(?:em|rem|%)/g)].map((m) => `${file}: ${m[0]}`),
      )
    expect(offenders).toEqual([])
  })

  it('圆角只取 token（50%、0 除外）', () => {
    const offenders = allStyleSources().flatMap(({ file, css }) =>
      [...css.matchAll(/border(?:-(?:top|bottom|left|right))?-radius:\s*[^;{}\r\n]*?\d+px/g)].map(
        (m) => `${file}: ${m[0]}`,
      ),
    )
    expect(offenders).toEqual([])
  })

  it('z-index 只取 7 档 token', () => {
    const offenders = allStyleSources().flatMap(({ file, css }) =>
      [...css.matchAll(/z-index:\s*\d+/g)].map((m) => `${file}: ${m[0]}`),
    )
    expect(offenders).toEqual([])
  })

  it('过渡只取 3 档时长与统一缓动', () => {
    const offenders = allStyleSources().flatMap(({ file, css }) =>
      [...css.matchAll(/transition:\s*[^;{}\r\n]+/g)]
        .map((m) => m[0])
        .filter(
          (decl) =>
            /\d+(?:\.\d+)?m?s\b/.test(decl.replace(/var\([^)]*\)/g, '')) ||
            /cubic-bezier\(|(?<![\w-])ease(?![\w-])/.test(decl) ||
            !decl.includes('var(--dur-') ||
            !decl.includes('var(--ease-out)'),
        )
        .map((decl) => `${file}: ${decl}`),
    )
    expect(offenders).toEqual([])
  })
})
