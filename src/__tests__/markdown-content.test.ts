import { mount } from '@vue/test-utils'
import { describe, it, expect } from 'vitest'
import MarkdownContent from '../components/common/MarkdownContent.vue'

describe('MarkdownContent', () => {
  it('null 内容渲染为空', () => {
    const wrapper = mount(MarkdownContent, { props: { content: null } })
    const el = wrapper.find('.markdown-body')
    expect(el.element.innerHTML).toBe('')
  })

  it('空字符串渲染为空', () => {
    const wrapper = mount(MarkdownContent, { props: { content: '' } })
    expect(wrapper.find('.markdown-body').element.innerHTML).toBe('')
  })

  it('渲染 Markdown 换行与列表', () => {
    const md = '- 项目一\n- 项目二'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).toContain('<ul>')
    expect(html).toContain('<li>项目一</li>')
    expect(html).toContain('<li>项目二</li>')
  })

  it('渲染链接为可点击的 <a>', () => {
    const md = '[官网](https://example.com)'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).toContain('<a href="https://example.com"')
    expect(html).toContain('>官网</a>')
  })

  it('渲染图片为 <img>', () => {
    const md = '![alt](https://example.com/x.png)'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    // 远程图片被改写为 media 网关地址（Rust 按代理设置下载）
    expect(html).toContain('<img src="http://media.localhost/' + encodeURIComponent('https://example.com/x.png') + '"')
    expect(html).toContain('alt="alt"')
  })

  it('渲染相对/自身资源图片不改写', () => {
    const md = '![本地](/icons.svg#x)'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).toContain('<img src="/icons.svg#x"')
  })

  it('保留内联 <br> 标签', () => {
    const md = '第一行<br>第二行'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).toContain('<br>')
  })

  it('清洗 <script> 标签（XSS 防护）', () => {
    const md = `<script>alert(1)<` + `/script>正常文本`
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).not.toContain('<script>')
    expect(html).not.toContain('alert(1)')
    expect(html).toContain('正常文本')
  })

  it('清洗事件处理器属性（XSS 防护）', () => {
    const md = '<a href="https://example.com" onclick="alert(1)">链接</a>'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).not.toContain('onclick')
    expect(html).not.toContain('alert')
    // href 保留
    expect(html).toContain('href="https://example.com"')
  })

  it('渲染代码块', () => {
    const md = '```\nconst x = 1\n```'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).toContain('<pre>')
    expect(html).toContain('<code>')
    expect(html).toContain('const x = 1')
  })

  it('渲染引用块', () => {
    const md = '> 这是引用'
    const wrapper = mount(MarkdownContent, { props: { content: md } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).toContain('<blockquote>')
    expect(html).toContain('这是引用')
  })

  it('noCache（流式路径）渲染结果与缓存路径一致', () => {
    const md = '- 项目一\n- 项目二'
    const cached = mount(MarkdownContent, { props: { content: md } })
    const uncached = mount(MarkdownContent, { props: { content: md, noCache: true } })
    expect(uncached.find('.markdown-body').element.innerHTML).toBe(
      cached.find('.markdown-body').element.innerHTML,
    )
  })

  it('noCache（流式路径）XSS 清洗仍生效', () => {
    const md = `<script>alert(1)<` + `/script>正常文本`
    const wrapper = mount(MarkdownContent, { props: { content: md, noCache: true } })
    const html = wrapper.find('.markdown-body').element.innerHTML
    expect(html).not.toContain('<script>')
    expect(html).not.toContain('alert(1)')
    expect(html).toContain('正常文本')
  })
})
