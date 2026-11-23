import { describe, it, expect, afterEach } from 'vitest'
import {
  splitByTokens,
  extractBodySnippet,
  collectQueryRanges,
  collectPhraseRanges,
  setActiveHighlightRange,
  clearActiveHighlight,
} from '../utils/searchHighlight'

describe('splitByTokens', () => {
  it('无 token 或无命中时整段返回', () => {
    expect(splitByTokens('hello world', [])).toEqual([{ text: 'hello world', hit: false }])
    expect(splitByTokens('hello world', ['xyz'])).toEqual([{ text: 'hello world', hit: false }])
  })

  it('大小写不敏感切分命中段', () => {
    expect(splitByTokens('Hello World', ['hello'])).toEqual([
      { text: 'Hello', hit: true },
      { text: ' World', hit: false },
    ])
  })

  it('多 token 与重叠/相邻区间合并', () => {
    const segs = splitByTokens('abcdef', ['abc', 'bcd', 'ef'])
    expect(segs).toEqual([{ text: 'abcdef', hit: true }])
    const apart = splitByTokens('abcXef', ['abc', 'ef'])
    expect(apart).toEqual([
      { text: 'abc', hit: true },
      { text: 'X', hit: false },
      { text: 'ef', hit: true },
    ])
  })

  it('同一 token 多次命中', () => {
    const segs = splitByTokens('aXaXa', ['x'])
    expect(segs.filter(s => s.hit)).toHaveLength(2)
    expect(segs.map(s => s.text).join('')).toBe('aXaXa')
  })
})

describe('extractBodySnippet', () => {
  it('可见字段已含全部命中词时返回 null', () => {
    expect(extractBodySnippet('修复 Kernel 崩溃', ['body text'], 'kernel')).toBeNull()
  })

  it('空查询返回 null', () => {
    expect(extractBodySnippet('visible', ['body'], '  ')).toBeNull()
  })

  it('正文命中时返回带高亮段的上下文', () => {
    const body = '前文'.repeat(20) + '修复了 kernel 崩溃问题' + '后文'.repeat(40)
    const segs = extractBodySnippet('摘要不含命中词', [body], 'kernel')
    expect(segs).not.toBeNull()
    expect(segs!.some(s => s.hit && s.text === 'kernel')).toBe(true)
    const text = segs!.map(s => s.text).join('')
    expect(text.startsWith('…')).toBe(true)
    expect(text.endsWith('…')).toBe(true)
  })

  it('原文未命中时回退到索引全文（小写兜底）', () => {
    const segs = extractBodySnippet('摘要', [null, '短预览无命中', 'deep body has kernel fix'], 'kernel')
    expect(segs).not.toBeNull()
    expect(segs!.some(s => s.hit)).toBe(true)
  })

  it('多词查询：可见字段缺任一词即出片段', () => {
    const segs = extractBodySnippet('只含 alpha', ['alpha and beta in body'], 'alpha beta')
    expect(segs).not.toBeNull()
    expect(segs!.filter(s => s.hit).map(s => s.text)).toEqual(['alpha', 'beta'])
  })

  it('多词查询优先定位可见字段缺失的词，不重复展示已可见的命中', () => {
    const body = 'kernel' + 'x'.repeat(120) + 'fix'
    const segs = extractBodySnippet('摘要里已有 kernel', [body], 'kernel fix')
    expect(segs).not.toBeNull()
    expect(segs!.filter(s => s.hit).map(s => s.text)).toEqual(['fix'])
  })

  it('候选正文只含已可见词时跳过，不产出无信息量片段', () => {
    expect(extractBodySnippet('摘要含 kernel', ['kernel only'], 'kernel fix')).toBeNull()
  })
})

describe('collectQueryRanges', () => {
  it('收集文本节点中的命中区间', () => {
    const el = document.createElement('div')
    el.innerHTML = '<span>Hello Kernel</span><button>kernel button</button>'
    const ranges = collectQueryRanges(el, ['kernel'], 'button')
    expect(ranges).toHaveLength(1)
    expect(ranges[0].toString()).toBe('Kernel')
  })

  it('无 token 返回空', () => {
    const el = document.createElement('div')
    el.textContent = 'kernel'
    expect(collectQueryRanges(el, [])).toHaveLength(0)
  })

  it('同一节点多次命中都收集', () => {
    const el = document.createElement('div')
    el.textContent = 'kernel and kernel'
    expect(collectQueryRanges(el, ['kernel'])).toHaveLength(2)
  })

  it('skipSelector 命中的祖先内文本不收集（如代码语言标签）', () => {
    const el = document.createElement('div')
    el.innerHTML = '<pre><code>kernel()</code><span class="md-code-lang">kernel</span></pre>'
    const ranges = collectQueryRanges(el, ['kernel'], 'button, .md-code-lang')
    expect(ranges.map(r => r.toString())).toEqual(['kernel'])
  })
})

describe('collectPhraseRanges', () => {
  it('按文档顺序匹配完整短语且大小写不敏感', () => {
    const el = document.createElement('div')
    el.innerHTML = '<p>Alpha Beta and alpha beta</p><p>ALPHA beta</p>'
    const ranges = collectPhraseRanges(el, 'alpha beta')
    expect(ranges.map(r => r.toString())).toEqual(['Alpha Beta', 'alpha beta', 'ALPHA beta'])
  })

  it('同一块内支持跨内联标签匹配', () => {
    const el = document.createElement('div')
    el.innerHTML = '<p>fix <code>foo</code> <strong>bar</strong> bug</p>'
    const ranges = collectPhraseRanges(el, 'fix foo bar')
    expect(ranges).toHaveLength(1)
    expect(ranges[0].toString()).toBe('fix foo bar')
  })

  it('跨块级元素与 <br> 不误连匹配', () => {
    const el = document.createElement('div')
    el.innerHTML = '<p>hello</p><p>world</p><p>foo<br>bar</p>'
    expect(collectPhraseRanges(el, 'helloworld')).toHaveLength(0)
    expect(collectPhraseRanges(el, 'foobar')).toHaveLength(0)
  })

  it('尊重 skipSelector 并忽略纯空白查询', () => {
    const el = document.createElement('div')
    el.innerHTML = '<pre><code>match</code><span class="md-code-lang">match</span><button>match</button></pre>'
    expect(collectPhraseRanges(el, 'match', 'button, .md-code-lang')).toHaveLength(1)
    expect(collectPhraseRanges(el, '   ')).toHaveLength(0)
  })

  it('skipSelector 夹在文本中间时会断开拼接，不跨跳过元素误匹配', () => {
    const el = document.createElement('div')
    el.innerHTML = '<p>foo<button>X</button>bar</p><p>baz<button></button>qux</p>'
    expect(collectPhraseRanges(el, 'foobar', 'button')).toHaveLength(0)
    expect(collectPhraseRanges(el, 'bazqux', 'button')).toHaveLength(0)
    expect(collectPhraseRanges(el, 'foo', 'button').map(r => r.toString())).toEqual(['foo'])
    expect(collectPhraseRanges(el, 'bar', 'button').map(r => r.toString())).toEqual(['bar'])
  })
})

describe('setActiveHighlightRange / clearActiveHighlight', () => {
  const originalCSS = globalThis.CSS
  const originalHighlight = globalThis.Highlight

  afterEach(() => {
    Object.defineProperty(globalThis, 'CSS', { value: originalCSS, configurable: true, writable: true })
    Object.defineProperty(globalThis, 'Highlight', { value: originalHighlight, configurable: true, writable: true })
  })

  it('登记当前命中项并设置高优先级，只有对应 owner 能清除', () => {
    const map = new Map<string, { priority?: number; ranges: Range[] }>()
    class FakeHighlight {
      priority = 0
      ranges: Range[]
      constructor(...ranges: Range[]) {
        this.ranges = ranges
      }
    }
    Object.defineProperty(globalThis, 'CSS', { value: { highlights: map }, configurable: true, writable: true })
    Object.defineProperty(globalThis, 'Highlight', { value: FakeHighlight, configurable: true, writable: true })

    const ownerA = {}
    const ownerB = {}
    const r = document.createRange()

    setActiveHighlightRange(ownerA, r)
    expect(map.get('search-hit-current')?.priority).toBe(1)
    expect(map.get('search-hit-current')?.ranges).toEqual([r])

    clearActiveHighlight(ownerB)
    expect(map.has('search-hit-current')).toBe(true)

    clearActiveHighlight(ownerA)
    expect(map.has('search-hit-current')).toBe(false)
  })
})
