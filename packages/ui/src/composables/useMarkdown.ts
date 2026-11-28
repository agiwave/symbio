/**
 * Markdown 渲染（模块级：不依赖组件实例，缓存才有处可挂）
 *
 * 对外只有 `renderMarkdownCached` 一个入口——会话流经 `useMessageContent` 调用它，
 * 渲染结果按内容缓存（流式期间同一份正文只渲染一次）。
 */

import { marked } from 'marked'
import { logger } from '@/utils/logger'

// 配置 marked - marked v17+ 使用对象配置
marked.setOptions({ 
  breaks: true, 
  gfm: true,
  async: false  // 禁用异步模式以获得同步返回
})

/** HTML 转义（代码块内容必须保持原文，不能当标签解析） */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/**
 * 围栏代码块：带上语言名（`data-lang`，由全局样式画成小标签）。
 *
 * 不引入语法高亮库：只把「这段是什么语言」这个**现成事实**显示出来，
 * 着色仍是纯 CSS（全局 `markdown.css`）。
 */
marked.use({
  renderer: {
    code({ text, lang }: { text: string; lang?: string }) {
      const language = (lang ?? '').trim().split(/\s+/)[0]
      const safeLang = language ? ` data-lang="${escapeHtml(language)}"` : ''
      return `<pre class="code-block"${safeLang}><code>${escapeHtml(text)}</code></pre>`
    },
  },
})

/**
 * 模块级渲染：不依赖组件实例，因此可被缓存命中（H6）
 *
 * 原先它是 `useMarkdown()` 里的闭包，每次调用 `useMarkdown()` 都新建一个函数，
 * 于是缓存无处可挂——把它提到模块作用域，缓存才有意义。
 */
function renderMarkdown(content: string): string {
  try {
    // marked v17+ 默认返回 Promise，但设置 async: false 后可以同步使用
    const result = marked(content)
    // 检查是否为 Promise，如果是则返回原始内容（不应发生）
    if (result instanceof Promise) {
      logger.warn('useMarkdown', 'Markdown returned Promise, returning raw content')
      return content
    }
    return sanitize(result as string)
  } catch (error) {
    logger.error('useMarkdown', 'Failed to render markdown:', error)
    return content
  }
}

/**
 * 消毒：Markdown 原文里允许内嵌 HTML（GFM 的合法部分），而正文来自 LLM /
 * 工具输出，是不可信内容——直接 `v-html` 等于让一段回答能在应用 webview 里
 * 执行脚本（`<img onerror>` 一类）。
 *
 * ## 为什么不用 DOMPurify
 *
 * 它依赖宿主 DOM 的实现细节：在本项目的 happy-dom 测试环境下会**静默丢掉块级
 * 元素**（`<pre>` / `<h1>` / `<div>`…），同一份输入在真实 WebView 与测试里得到
 * 两种结果。安全件的行为必须与运行环境无关，否则测试锁定的是另一份现实。
 *
 * ## 为什么不用正则
 *
 * 正则无法正确理解 HTML 的解析边界（嵌套、属性引号、命名空间），白名单只能建在
 * **解析后的 DOM** 上。这里先用 `DOMParser` 建树，再按标签/属性白名单剪枝，
 * 最后序列化——解析与判断分两步，与浏览器实际接受的树是同一棵。
 *
 * 白名单是**默认拒绝**：未列出的标签拆掉外壳只留子节点，危险标签（脚本 / 样式 /
 * 命名空间 / 表单）整棵删除，未列出的属性一律丢弃。
 */

/** 保留的标签（排版类；不含任何可执行 / 可发送内容的元素） */
const ALLOWED_TAGS = new Set([
  'a', 'abbr', 'b', 'blockquote', 'br', 'code', 'dd', 'del', 'details', 'div', 'dl',
  'dt', 'em', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'img', 'ins', 'kbd',
  'li', 'ol', 'p', 'pre', 's', 'span', 'strong', 'sub', 'summary', 'sup', 'table',
  'tbody', 'td', 'tfoot', 'th', 'thead', 'tr', 'u', 'ul',
])

/**
 * 整棵删除的标签。
 *
 * `style` / `script` / 命名空间（`svg` / `math`）是 mXSS 的主要载体；`form` /
 * `input` / `iframe` / `object` / `embed` 能把正文变成可交互 / 可外送的东西。
 */
const DROP_TAGS = new Set([
  'script', 'style', 'iframe', 'object', 'embed', 'link', 'meta', 'base', 'form',
  'input', 'button', 'select', 'option', 'textarea', 'svg', 'math', 'template',
  'noscript', 'video', 'audio', 'source', 'track', 'canvas',
])

/** 保留的属性（其余全部丢弃；`on*` 永远不会出现在这里） */
const ALLOWED_ATTRS = new Set([
  'href', 'src', 'alt', 'title', 'class', 'colspan', 'rowspan', 'align', 'start',
  'data-lang',
])

/** 协议白名单：链接与图片来源只接受这些前缀 */
function safeUrl(raw: string): boolean {
  const v = raw.trim().toLowerCase()
  if (v.startsWith('http://') || v.startsWith('https://') || v.startsWith('mailto:')) return true
  // 站内相对地址 / 锚点
  if (v.startsWith('/') || v.startsWith('#') || v.startsWith('./') || v.startsWith('../')) return true
  // 图片内联是合法的（聊天里贴图就靠它），但只允许 image 类型
  if (v.startsWith('data:image/')) return true
  return false
}

/** 剪枝：按白名单递归处理一棵子树 */
function cleanNode(node: Node): void {
  const children = Array.from(node.childNodes)
  for (const child of children) {
    if (child.nodeType !== 1) continue // 文本 / 注释：文本保留，注释忽略
    const el = child as Element
    const tag = el.tagName.toLowerCase()
    if (DROP_TAGS.has(tag)) {
      el.remove()
      continue
    }
    cleanNode(el)
    if (!ALLOWED_TAGS.has(tag)) {
      // 未登记的标签：拆掉外壳，内容原样留下（不把一段正文连带丢掉）
      el.replaceWith(...Array.from(el.childNodes))
      continue
    }
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase()
      const isUrl = name === 'href' || name === 'src'
      if (!ALLOWED_ATTRS.has(name) || (isUrl && !safeUrl(attr.value))) {
        el.removeAttribute(attr.name)
      }
    }
  }
}

function sanitize(html: string): string {
  // 无 DOM 的环境无法建树，也就无法判断。正文来自 LLM / 工具输出（不可信），
  // 没有判断能力时一律不渲染 HTML —— 与下方 catch 分支的安全策略一致。
  // 消毒主路径由 happy-dom 单测覆盖（useMarkdown.spec.ts 文件头标注），
  // 这个分支只在真正的非 DOM 宿主里兜底，不再为单测让路。
  if (typeof DOMParser === 'undefined') return escapeHtml(html)
  try {
    const doc = new DOMParser().parseFromString(html, 'text/html')
    cleanNode(doc.body)
    return doc.body.innerHTML
  } catch (error) {
    // 消毒器自身异常时宁可不渲染 HTML：回退成纯文本，而不是把原串放行
    logger.error('useMarkdown', 'Sanitize failed, falling back to plain text:', error)
    return escapeHtml(html)
  }
}

/**
 * 渲染缓存上限（条）。流式期间正文每来一个 token 就变一次，缓存的价值在于
 * 「滚回去、切回来、重渲染」这些**重复内容**，而不是给每一帧都留一份。
 */
const CACHE_LIMIT = 150

/** 超过这个长度不进缓存：长文单次渲染，缓存它只会把小条目挤出去 */
const CACHE_MAX_CHARS = 2000

const cache = new Map<string, string>()

/** 带缓存的 Markdown 渲染（同一个 `content` 只渲染一次） */
export function renderMarkdownCached(content: string): string {
  if (!content) return ''
  if (content.length > CACHE_MAX_CHARS) return renderMarkdown(content)

  const hit = cache.get(content)
  if (hit !== undefined) return hit

  const html = renderMarkdown(content)
  if (cache.size >= CACHE_LIMIT) {
    // Map 的迭代序 = 插入序：删掉最老的一批（简单 LRU，够用）
    const oldest = cache.keys().next().value
    if (oldest !== undefined) cache.delete(oldest)
  }
  cache.set(content, html)
  return html
}

