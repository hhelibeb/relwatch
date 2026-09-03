<script lang="ts">
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { toMediaUrl } from '../../utils/imageProxy'

// marked 配置：关闭 mangle/escaping 由 DOMPurify 统一清洗
marked.setOptions({
  breaks: true,      // 单换行转 <br>，符合 release note 阅读习惯
  gfm: true,         // GitHub Flavored Markdown
})

// 远程图片一律改走 media 网关：CSP 的 img-src 已不放行任意 https: 图片源（收紧后
// 漏网远程图显式失败），media 协议由 Rust 按应用代理设置下载返回。此 hook 在
// DOMPurify 清洗后统一改写存活节点的 src，比在 marked 层做字符串替换可靠——
// 清洗后仍在的 <img> 一定带合法 src（onerror 等已被剥离）。
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'IMG') {
    const src = node.getAttribute('src')
    if (src) {
      node.setAttribute('src', toMediaUrl(src))
    }
  }
})

// 渲染结果缓存（模块级共享）：虚拟滚动中行卸载时实例级缓存会随之销毁，
// 滚动回来是全新实例，模块级缓存才能让"同一 content 复用清洗结果"真正生效，
// 避免重复 marked.parse + DOMPurify.sanitize
const CACHE_MAX = 100
const htmlCache = new Map<string, string>()

function renderMarkdownHtml(content: string): string {
  const cached = htmlCache.get(content)
  if (cached !== undefined) return cached
  const sanitized = parseAndSanitize(content)
  htmlCache.set(content, sanitized)
  // FIFO 淘汰最旧条目，防止缓存无限增长
  if (htmlCache.size > CACHE_MAX) {
    const oldest = htmlCache.keys().next().value
    if (oldest !== undefined) htmlCache.delete(oldest)
  }
  return sanitized
}

/** 流式渲染路径：不读不写缓存。流式期间内容每个合帧批次都不同，
 *  写入只会以「内容前缀」形式灌满并冲刷掉列表/详情等静态场景的缓存
 *  （FIFO 100 条撑不过一次长输出的 delta 数），读则永远 miss。 */
function renderMarkdownHtmlUncached(content: string): string {
  return parseAndSanitize(content)
}

/** parse + 清洗（缓存读写之外的可复用部分）。 */
function parseAndSanitize(content: string): string {
  const raw = marked.parse(content, { async: false }) as string
  // DOMPurify 清洗：移除 script/事件处理器等危险内容，保留常见 Markdown 渲染产物
  const sanitized = DOMPurify.sanitize(raw, {
    ALLOWED_TAGS: [
      'p', 'br', 'hr', 'strong', 'em', 'del', 'code', 'pre', 'blockquote',
      'ul', 'ol', 'li', 'a', 'img', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
      'table', 'thead', 'tbody', 'tr', 'th', 'td', 'span', 'div', 'sup', 'sub',
    ],
    ALLOWED_ATTR: ['href', 'src', 'alt', 'title', 'class', 'target', 'rel'],
  })
  return sanitized
}
</script>

<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ content: string | null; noCache?: boolean }>()

const html = computed(() => {
  if (!props.content) return ''
  return props.noCache ? renderMarkdownHtmlUncached(props.content) : renderMarkdownHtml(props.content)
})
</script>

<template>
  <div class="markdown-body" v-html="html"></div>
</template>

<style scoped>
.markdown-body {
  font-size: 13px;
  line-height: 1.6;
  color: var(--text);
  word-break: break-word;
}

.markdown-body :deep(p) {
  margin: 6px 0;
}

.markdown-body :deep(p:first-child) {
  margin-top: 0;
}

.markdown-body :deep(p:last-child) {
  margin-bottom: 0;
}

.markdown-body :deep(h1),
.markdown-body :deep(h2),
.markdown-body :deep(h3),
.markdown-body :deep(h4),
.markdown-body :deep(h5),
.markdown-body :deep(h6) {
  margin: 12px 0 6px;
  font-weight: 600;
  line-height: 1.3;
}

.markdown-body :deep(h1) { font-size: 1.3em; }
.markdown-body :deep(h2) { font-size: 1.2em; }
.markdown-body :deep(h3) { font-size: 1.1em; }
.markdown-body :deep(h4),
.markdown-body :deep(h5),
.markdown-body :deep(h6) { font-size: 1em; }

.markdown-body :deep(a) {
  color: var(--primary);
  text-decoration: none;
}

.markdown-body :deep(a:hover) {
  text-decoration: underline;
}

.markdown-body :deep(ul),
.markdown-body :deep(ol) {
  margin: 6px 0;
  padding-left: 22px;
}

.markdown-body :deep(li) {
  margin: 2px 0;
}

.markdown-body :deep(code) {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.9em;
  padding: 1px 4px;
  background: var(--bg-subtle);
  border-radius: var(--radius-xs);
}

.markdown-body :deep(pre) {
  margin: 8px 0;
  padding: 10px 12px;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
  overflow-x: auto;
}

.markdown-body :deep(pre code) {
  padding: 0;
  background: transparent;
  font-size: 12px;
  line-height: 1.5;
}

.markdown-body :deep(blockquote) {
  margin: 8px 0;
  padding: 4px 12px;
  border-left: 3px solid var(--border);
  color: var(--text-muted);
}

.markdown-body :deep(img) {
  max-width: 100%;
  height: auto;
  border-radius: 6px;
}

.markdown-body :deep(table) {
  border-collapse: collapse;
  margin: 8px 0;
  font-size: 12px;
}

.markdown-body :deep(th),
.markdown-body :deep(td) {
  border: 1px solid var(--border);
  padding: 4px 8px;
}

.markdown-body :deep(th) {
  background: var(--bg-subtle);
  font-weight: 600;
}

.markdown-body :deep(hr) {
  border: none;
  border-top: 1px solid var(--border);
  margin: 10px 0;
}
</style>
