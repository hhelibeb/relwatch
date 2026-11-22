import { describe, it, expect } from 'vitest'
import { splitByTokens, extractBodySnippet, collectQueryRanges } from '../utils/searchHighlight'

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