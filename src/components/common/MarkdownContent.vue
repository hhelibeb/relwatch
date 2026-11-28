<script lang="ts">
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { toMediaUrl } from '../../utils/imageProxy'

// marked 配置（渲染产物统一由 DOMPurify 清洗）
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
import { computed, ref, watch, onMounted, onBeforeUnmount } from 'vue'
import hljs from 'highlight.js/lib/common'
import { copyTextToClipboard } from '../../api/client'
import { useUiStore } from '../../stores/ui'
import { extractErrorMessage } from '../../services/feedback'
import { t } from '../../i18n'

const props = defineProps<{ content: string | null; noCache?: boolean }>()
const ui = useUiStore()
const showToast = (msg: string) => ui.showToast(msg)

const html = computed(() => {
  if (!props.content) return ''
  return props.noCache ? renderMarkdownHtmlUncached(props.content) : renderMarkdownHtml(props.content)
})

// ── 代码块增强：语法高亮 + 语言标签 + 复制按钮 ──────────────
// 在渲染后的 DOM 上做（而非 HTML 字符串层）：清洗缓存保持与内容一一对应，
// 高亮/按钮是展示层增强，v-html 每次整体替换后重新施加。
const rootEl = ref<HTMLElement | null>(null)
const copyTimers = new Set<ReturnType<typeof setTimeout>>()
// 超大代码块不做高亮（自动检测尤其贵），复制按钮不受影响
const HIGHLIGHT_MAX_CHARS = 30000
const AUTO_DETECT_MAX_CHARS = 5000

const COPY_ICON = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2"/></svg>'
const CHECK_ICON = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 8.5l3 3 6-7"/></svg>'

function highlightCode(code: HTMLElement): string | null {
  const text = code.textContent ?? ''
  if (!text || text.length > HIGHLIGHT_MAX_CHARS || code.dataset.highlighted) return null
  const explicit = code.className.match(/language-([\w+-]+)/)?.[1]
  try {
    if (explicit) {
      // 未注册的语言（mermaid 等）保持原样，不做无意义的自动猜测
      if (!hljs.getLanguage(explicit)) return null
      const result = hljs.highlight(text, { language: explicit, ignoreIllegals: true })
      code.innerHTML = result.value
      code.classList.add('hljs')
      code.dataset.highlighted = 'yes'
      return result.language ?? explicit
    }
    if (text.length > AUTO_DETECT_MAX_CHARS) return null
    const result = hljs.highlightAuto(text)
    if (!result.language) return null
    code.innerHTML = result.value
    code.classList.add('hljs')
    code.dataset.highlighted = 'yes'
    return result.language
  } catch {
    return null
  }
}

async function handleCopyCode(code: HTMLElement, btn: HTMLButtonElement) {
  try {
    // marked 的围栏代码块 textContent 带尾随换行，复制时去掉（与 GitHub 复制行为一致）
    await copyTextToClipboard((code.textContent ?? '').replace(/\n$/, ''))
    btn.classList.add('copied')
    btn.innerHTML = CHECK_ICON
    btn.title = t('release.copied')
    const timer = setTimeout(() => {
      copyTimers.delete(timer)
      if (!btn.isConnected) return
      btn.classList.remove('copied')
      btn.innerHTML = COPY_ICON
      btn.title = t('context.copy_code')
    }, 1600)
    copyTimers.add(timer)
  } catch (e: unknown) {
    showToast(t('release.copy_failed') + extractErrorMessage(e))
  }
}

function enhanceCodeBlocks() {
  const root = rootEl.value
  if (!root) return
  for (const pre of root.querySelectorAll('pre')) {
    if (pre.parentElement?.classList.contains('md-code-wrap')) continue
    const code = pre.querySelector('code')
    if (!code) continue

    // pre 自身承担横向滚动：工具条若放在 pre 内会跟着代码一起滚走（长行右滚后
    // 复制按钮不可见），故包一层不滚动的定位容器，工具条挂在容器上。
    const wrap = document.createElement('div')
    wrap.className = 'md-code-wrap'
    pre.parentNode?.insertBefore(wrap, pre)
    wrap.appendChild(pre)

    const lang = highlightCode(code)
    pre.classList.add('md-code-block')
    const label = lang ?? code.className.match(/language-([\w+-]+)/)?.[1] ?? null
    if (label) {
      const langEl = document.createElement('span')
      langEl.className = 'md-code-lang'
      langEl.textContent = label
      wrap.appendChild(langEl)
    }
    const btn = document.createElement('button')
    btn.type = 'button'
    btn.className = 'md-code-copy'
    btn.title = t('context.copy_code')
    btn.innerHTML = COPY_ICON
    btn.addEventListener('click', () => { void handleCopyCode(code, btn) })
    wrap.appendChild(btn)
  }
}

watch(html, enhanceCodeBlocks, { flush: 'post' })
onMounted(enhanceCodeBlocks)
onBeforeUnmount(() => {
  for (const timer of copyTimers) clearTimeout(timer)
  copyTimers.clear()
})
</script>

<template>
  <div ref="rootEl" class="markdown-body" v-html="html"></div>
</template>

<style scoped>
.markdown-body {
  font-size: var(--fs-body);
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
  font-family: var(--mono-font);
  font-size: 0.9em;
  padding: 1px 4px;
  background: var(--bg-subtle);
  border-radius: var(--radius-xs);
}

.markdown-body :deep(.md-code-wrap) {
  position: relative;
  margin: 8px 0;
}

.markdown-body :deep(pre) {
  margin: 0;
  padding: 10px 12px;
  background: var(--bg-subtle);
  border-radius: var(--radius-sm);
  overflow-x: auto;
}

.markdown-body :deep(pre code) {
  padding: 0;
  background: transparent;
  font-size: var(--fs-meta);
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
  border-radius: var(--radius);
}

.markdown-body :deep(table) {
  border-collapse: collapse;
  margin: 8px 0;
  font-size: var(--fs-meta);
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

/* ── 代码块工具条：语言标签 + 复制按钮 ─────────────── */
.markdown-body :deep(.md-code-lang) {
  position: absolute;
  top: 7px;
  right: 36px;
  font-family: var(--mono-font);
  font-size: var(--fs-caption);
  color: var(--text-muted);
  user-select: none;
  pointer-events: none;
  transition: opacity var(--dur-hover) var(--ease-out);
}

.markdown-body :deep(.md-code-copy) {
  position: absolute;
  top: 4px;
  right: 4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text-muted);
  cursor: pointer;
  transition: opacity var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out), border-color var(--dur-hover) var(--ease-out);
}

.markdown-body :deep(.md-code-copy svg) {
  width: 13px;
  height: 13px;
}

.markdown-body :deep(.md-code-copy:hover) {
  color: var(--text);
  border-color: var(--border-strong);
}

.markdown-body :deep(.md-code-copy.copied) {
  color: var(--success);
  border-color: var(--success);
}

/* 有 hover 能力时工具条默认收起，悬停/键盘聚焦浮现；触摸设备常驻 */
@media (hover: hover) {
  .markdown-body :deep(.md-code-copy),
  .markdown-body :deep(.md-code-lang) {
    opacity: 0;
  }

  .markdown-body :deep(.md-code-wrap:hover .md-code-copy),
  .markdown-body :deep(.md-code-wrap:hover .md-code-lang),
  .markdown-body :deep(.md-code-wrap:focus-within .md-code-copy),
  .markdown-body :deep(.md-code-wrap:focus-within .md-code-lang) {
    opacity: 1;
  }
}

/* ── 语法高亮：取图表系列色板，深浅主题各自有值 ─────── */
.markdown-body :deep(.hljs-comment),
.markdown-body :deep(.hljs-quote) {
  color: var(--text-muted);
  font-style: italic;
}

.markdown-body :deep(.hljs-keyword),
.markdown-body :deep(.hljs-selector-tag),
.markdown-body :deep(.hljs-meta),
.markdown-body :deep(.hljs-doctag) {
  color: var(--data-series-4);
}

.markdown-body :deep(.hljs-string),
.markdown-body :deep(.hljs-regexp),
.markdown-body :deep(.hljs-addition) {
  color: var(--data-series-5);
}

.markdown-body :deep(.hljs-number),
.markdown-body :deep(.hljs-literal),
.markdown-body :deep(.hljs-symbol),
.markdown-body :deep(.hljs-bullet) {
  color: var(--data-series-3);
}

.markdown-body :deep(.hljs-title),
.markdown-body :deep(.hljs-section),
.markdown-body :deep(.hljs-name),
.markdown-body :deep(.hljs-selector-id),
.markdown-body :deep(.hljs-selector-class) {
  color: var(--data-series-2);
}

.markdown-body :deep(.hljs-attr),
.markdown-body :deep(.hljs-attribute),
.markdown-body :deep(.hljs-variable),
.markdown-body :deep(.hljs-template-variable),
.markdown-body :deep(.hljs-type),
.markdown-body :deep(.hljs-built_in) {
  color: var(--data-series-1);
}

.markdown-body :deep(.hljs-deletion) {
  color: var(--danger);
}

.markdown-body :deep(.hljs-emphasis) {
  font-style: italic;
}

.markdown-body :deep(.hljs-strong) {
  font-weight: 600;
}
</style>
