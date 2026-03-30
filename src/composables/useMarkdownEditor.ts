import { keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter } from '@codemirror/view'
import { EditorState } from '@codemirror/state'
import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
import { languages } from '@codemirror/language-data'
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { syntaxHighlighting, indentOnInput, bracketMatching, foldGutter, defaultHighlightStyle } from '@codemirror/language'
import { oneDark } from '@codemirror/theme-one-dark'
import type { Extension } from '@codemirror/state'

export interface CodeBlock {
  id: string
  language: string
  code: string
  from: number
  to: number
  executable: boolean
}

/**
 * 从 Markdown 内容中提取代码块
 */
export function extractCodeBlocks(content: string): CodeBlock[] {
  const blocks: CodeBlock[] = []
  const codeBlockRegex = /```(\w+)\s*\n([\s\S]*?)```/g
  
  let match
  while ((match = codeBlockRegex.exec(content)) !== null) {
    const language = match[1].toLowerCase()
    const code = match[2] || ''
    const executable = match[1].includes('run')
    
    blocks.push({
      id: `block-${blocks.length}-${Date.now()}`,
      language,
      code: code.trim(),
      from: match.index,
      to: match.index + match[0].length,
      executable,
    })
  }

  return blocks
}

/**
 * 判断语言是否可执行
 */
export function isExecutableLanguage(language: string): boolean {
  const executableLanguages = ['bash', 'sh', 'shell', 'r', 'python', 'python3', 'perl', 'ruby']
  return executableLanguages.includes(language.toLowerCase())
}

/**
 * 创建编辑器扩展配置
 */
export function createEditorExtensions(theme: 'light' | 'dark' = 'light'): Extension[] {
  const extensions: Extension[] = [
    lineNumbers(),
    highlightActiveLine(),
    highlightActiveLineGutter(),
    history(),
    foldGutter(),
    indentOnInput(),
    bracketMatching(),
    markdown({
      base: markdownLanguage,
      codeLanguages: languages,
    }),
    syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
    keymap.of([...defaultKeymap, ...historyKeymap]),
  ]

  if (theme === 'dark') {
    extensions.push(oneDark)
  }

  return extensions
}