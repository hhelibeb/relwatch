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
  '--hover-surface',
  '--hover-surface-nested',
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

/**
 * 列表 hover 语言守卫。
 *
 * 背景：版本卡片靠边框加深、源列表靠背景抬升、日志行干脆没有 hover，
 * 三种列表三套反馈，扫长列表时手感不一致。收敛为「背景抬升 + 120ms」后，
 * 用这条钉住三处不再分叉——尤其防止有人回头把 hover 又写成 border-color。
 * 抬升色走 --hover-surface 而不是 --bg-subtle：后者深色值只比 --surface 高 1.03:1，
 * 深色主题下整行划过等于没反馈（见 style.css 注释）。
 */
describe('列表 hover 语言守卫', () => {
  const LIST_HOVER = [
    { file: 'components/ReleaseItem.vue', selector: '.release-item:hover' },
    { file: 'components/ReleaseAggregatedList.vue', selector: '.repo-group-body .release-item:hover' },
    { file: 'components/ReleaseAggregatedList.vue', selector: '.repo-group-header:hover' },
    { file: 'components/SourceTab.vue', selector: '.source-item:hover' },
    { file: 'components/LogTab.vue', selector: '.log-item:hover' },
  ]

  const normalized = () =>
    allStyleSources().map(({ file, css }) => ({ file: file.replace(/\\/g, '/'), css }))

  const source = (target: string) => {
    const hit = normalized().find(({ file }) => file.endsWith(target))
    expect(hit, `找不到样式源: ${target}`).toBeTruthy()
    return hit?.css ?? ''
  }

  const hoverRule = (css: string, selector: string) => {
    const start = css.indexOf(`${selector} {`)
    if (start === -1) return null
    return css.slice(start, css.indexOf('}', start))
  }

  it('列表行 hover 都存在，且统一抬背景', () => {
    for (const { file, selector } of LIST_HOVER) {
      const rule = hoverRule(source(file), selector)
      expect(rule, `${file} 缺少 ${selector}`).toBeTruthy()
      expect(rule, `${file} hover 未抬背景`).toContain('background: var(--hover-surface)')
    }
  })

  it('hover 不改边框色', () => {
    const offenders = LIST_HOVER.filter(({ file, selector }) => (hoverRule(source(file), selector) ?? '').includes('border-color'))
    expect(offenders.map((o) => o.file)).toEqual([])
  })

  // 深色下 --bg-subtle 与 --surface 只差 1.03:1：任何 hover 复用它都会变成"看不见的反馈"。
  it('hover 一律用 --hover-surface，不用 --bg-subtle', () => {
    const block = /([^{}]+)\{([^{}]*)\}/g
    const offenders = allStyleSources().flatMap(({ file, css }) =>
      [...css.matchAll(block)]
        .filter((m) => m[1].includes(':hover') && m[2].includes('var(--bg-subtle)'))
        .map((m) => `${file}: ${m[1].trim().replace(/\s+/g, ' ')}`),
    )
    expect(offenders).toEqual([])
  })

  /**
   * 嵌套悬停面。
   *
   * 行/卡片 hover 抬到 --hover-surface 之后，容器内的可点块若还用同一档，
   * 指针压上去看到的是容器自己的底色，等于没有反馈（深色下改复用 --bg-hover
   * 也只有 1.05:1）。故单开一档，并用数值钉住「比一级高一档」这个关系：
   * 只断言用了哪个 token，拦不住有人把两个色值写反。
   */
  const HOVER_TOKENS = { base: '--hover-surface', nested: '--hover-surface-nested' }

  const tokenValue = (themeBlock: string, token: string) => {
    const from = STYLE_CSS.indexOf(themeBlock)
    const body = STYLE_CSS.slice(from, STYLE_CSS.indexOf('}', from))
    return body.match(new RegExp(`${token}\\s*:\\s*(#[0-9a-fA-F]{6})`))?.[1] ?? ''
  }

  const luminance = (hex: string) => {
    const channels = [1, 3, 5].map((i) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
    })
    return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
  }

  const contrast = (a: string, b: string) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
    return (hi + 0.05) / (lo + 0.05)
  }

  it('二级悬停面在深浅两套都比一级高一档', () => {
    for (const theme of [':root {', '[data-theme="dark"] {']) {
      const base = tokenValue(theme, HOVER_TOKENS.base)
      const nested = tokenValue(theme, HOVER_TOKENS.nested)
      expect(base && nested, `${theme} 缺少悬停面 token`).toBeTruthy()
      const step = contrast(base, nested)
      expect(step, `${theme} 二级与一级拉不开（${step.toFixed(3)}:1）`).toBeGreaterThan(1.08)
      expect(step, `${theme} 二级过重（${step.toFixed(3)}:1）`).toBeLessThan(1.25)
    }
  })

  it('容器内的可点块用二级悬停面', () => {
    const NESTED_HOVER = [
      { file: 'components/ReleaseItem.vue', selector: '.yt-desc:hover' },
      { file: 'components/ReleaseItem.vue', selector: '.release-body-text:hover' },
      { file: 'components/ReleaseItem.vue', selector: '.release-link-action:hover' },
      { file: 'components/ReleaseItem.vue', selector: '.release-item .btn-sm:hover' },
      { file: 'components/SourceTab.vue', selector: '.btn-icon-action:hover:not(:disabled)' },
      { file: 'components/SourceTab.vue', selector: '.btn-check:hover:not(:disabled)' },
    ]

    for (const { file, selector } of NESTED_HOVER) {
      const css = source(file)
      const start = css.indexOf(selector)
      expect(start, `${file} 缺少 ${selector}`).toBeGreaterThan(-1)
      const open = css.indexOf('{', start)
      const rule = css.slice(open, css.indexOf('}', open))
      expect(rule, `${file} ${selector} 未用二级悬停面`).toContain(
        `background: var(${HOVER_TOKENS.nested})`,
      )
    }
  })
})
