<template>
  <Teleport to="body">
    <div
      v-if="store.isOpen"
      class="file-viewer-overlay"
      role="dialog"
      aria-modal="true"
      @keydown.esc="onCloseRequest"
    >
      <header class="overlay-header">
        <div class="header-left">
          <span class="file-icon">{{ icon }}</span>
          <span class="file-name" :title="store.path || ''">{{ store.name }}</span>
          <span v-if="dirty" class="dirty-mark" title="未保存">●</span>
        </div>

        <div class="header-center">
          <!-- 视图切换：仅 markdown 出现 -->
          <div v-if="isMarkdown" class="view-switch">
            <button
              class="switch-btn"
              :class="{ active: viewMode === 'edit' }"
              @click="viewMode = 'edit'"
              title="编辑"
            >编辑</button>
            <button
              class="switch-btn"
              :class="{ active: viewMode === 'preview' }"
              @click="viewMode = 'preview'"
              title="预览"
            >预览</button>
          </div>
        </div>

        <div class="header-right">
          <span v-if="saveStatus === 'saving'" class="save-status">保存中…</span>
          <span v-else-if="saveStatus === 'saved'" class="save-status saved">已保存</span>
          <span v-else-if="saveStatus === 'error'" class="save-status error">保存失败</span>

          <button
            v-if="canEdit"
            class="icon-btn save-btn"
            :disabled="!dirty || saveStatus === 'saving'"
            :title="dirty ? '保存 (Ctrl+S)' : '未修改'"
            @click="onSave"
          >
            <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
              <polyline points="17 21 17 13 7 13 7 21" />
              <polyline points="7 3 7 8 15 8" />
            </svg>
          </button>

          <button class="icon-btn" @click="onCloseRequest" title="关闭 (Esc)">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      </header>

      <div class="overlay-body">
        <div v-if="loading" class="loading-state">
          <div class="spinner" />
          <p>加载中…</p>
        </div>

        <div v-else-if="loadError" class="error-state">
          <p>加载失败：{{ loadError }}</p>
        </div>

        <!-- 二进制/无扩展名：只读展示 -->
        <div v-else-if="!isTextLike" class="binary-state">
          <p>该文件类型不支持编辑/预览。</p>
          <p v-if="content" class="binary-info">文件大小：{{ content.length }} 字节</p>
        </div>

        <!-- Markdown 预览模式 -->
        <div
          v-else-if="isMarkdown && viewMode === 'preview'"
          class="md-content markdown-body"
          v-html="renderedMarkdown"
        />

        <!-- 编辑模式（默认）：CodeEditor -->
        <div v-else class="editor-wrapper" @contextmenu.prevent>
          <CodeEditor
            ref="editorRef"
            :model-value="content"
            :file-path="store.path ?? undefined"
            @update:model-value="onContentChange"
            @request-save="onSave"
            @selection-change="onSelectionChange"
          />

          <!-- 选中时浮出"发送到 AI"—— 定位在编辑器区域底部右下角，
               避开 header（工具栏/关闭按钮所在区域） -->
          <Transition name="fade-pop">
            <button
              v-if="selectionInfo"
              class="send-to-ai"
              :style="floatingBtnStyle"
              title="将选中文本作为上下文发送给 AI"
              @click="onSendToAI"
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" />
              </svg>
              <span>发送到 AI</span>
              <span v-if="selectionInfo.endLine > selectionInfo.startLine" class="line-badge">
                L{{ selectionInfo.startLine }}-{{ selectionInfo.endLine }}
              </span>
              <span v-else class="line-badge">L{{ selectionInfo.startLine }}</span>
            </button>
          </Transition>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
/**
 * 全屏文件查看器 / 编辑器
 *
 * 能力：
 * 1. 编辑：默认进入可编辑模式（CodeEditor 通用文本/代码/Markdown）
 * 2. 保存：Ctrl/Cmd+S 或工具栏保存按钮；防抖期间禁用按钮
 * 3. Markdown 预览：额外提供"预览"切换
 * 4. AI 选区：编辑器选中文本后，浮出"发送到 AI"按钮
 *    点击 → 写入全局 AI 上下文 + 通过 enqueueInputInject 把
 *           摘要文本塞到**当前活跃会话**的 AI 输入框，
 *           然后关闭覆盖层，让用户看到聊天窗口聚焦输入
 */
import { ref, computed, watch, onBeforeUnmount, nextTick } from 'vue'
import { useFileViewerStore } from '@/stores/fileViewer'
import { useExplorerStore } from '@/stores/explorer'
import { useSessionsStore } from '@/stores/sessions'
import { useMarkdown } from '@/composables/useMarkdown'
import { setModelContext, requestFocusInput } from '@/composables/useModelContext'
import { callPlugin } from '@/services/plugin'
import CodeEditor from '@/components/CodeEditor.vue'
import { logger } from '@/utils/logger'

type SelectionInfo = { text: string; startLine: number; endLine: number }
type ViewMode = 'edit' | 'preview'
type SaveStatus = 'idle' | 'saving' | 'saved' | 'error'

const store = useFileViewerStore()
const explorer = useExplorerStore()
const sessionsStore = useSessionsStore()
const { renderMarkdown } = useMarkdown()

const content = ref('')
const originalContent = ref('') // 用于计算 dirty
const loading = ref(false)
const loadError = ref<string | null>(null)
const editorRef = ref<InstanceType<typeof CodeEditor> | null>(null)
const viewMode = ref<ViewMode>('edit')
const saveStatus = ref<SaveStatus>('idle')
const saveErrorMsg = ref<string>('')

// 选区信息（编辑器上报）
const selectionInfo = ref<SelectionInfo | null>(null)
// 选区在视口的位置（用于浮动按钮定位）
const selectionRect = ref<DOMRect | null>(null)

const TEXT_RE = /\.(txt|json|ya?ml|toml|js|ts|tsx|jsx|vue|rs|py|java|go|c|cpp|h|hpp|cs|rb|php|sh|bash|sql|html|css|scss|less|xml|ini|log|env|conf|cfg|gradle|kt|swift|md|markdown)$/i
const isMarkdown = computed(() => /\.(md|markdown)$/i.test(store.path || ''))
const isTextLike = computed(() => TEXT_RE.test(store.path || ''))
const canEdit = computed(() => isTextLike.value)

const icon = computed(() => {
  if (isMarkdown.value) return '📝'
  if (isTextLike.value) return '📜'
  return '📄'
})

const renderedMarkdown = computed(() => {
  if (!isMarkdown.value) return ''
  return renderMarkdown(content.value)
})

const dirty = computed(() => content.value !== originalContent.value)

// 浮动按钮定位：固定贴在编辑器底部右下角，避开 header 与关闭按钮
const floatingBtnStyle = computed(() => ({
  bottom: '1.25rem',
  right: '1.25rem'
}))

function readResponseToString(result: any): string {
  if (result == null) return ''
  if (typeof result === 'string') return result
  if (typeof result.content === 'string') return result.content
  if (result.data) {
    if (typeof result.data === 'string') return result.data
    if (typeof result.data.content === 'string') return result.data.content
  }
  return JSON.stringify(result, null, 2)
}

async function load() {
  if (!store.path) return
  loading.value = true
  loadError.value = null
  content.value = ''
  originalContent.value = ''
  try {
    const result = await callPlugin<any, { path: string; workdir?: string }>('explorer/read', {
      path: store.path,
      workdir: store.workdir || undefined
    })
    const text = readResponseToString(result)
    content.value = text
    originalContent.value = text
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e)
    logger.error('FileViewerOverlay', `读取文件失败: ${store.path}`, e)
  } finally {
    loading.value = false
  }
}

function onContentChange(v: string) {
  content.value = v
  if (saveStatus.value === 'saved') saveStatus.value = 'idle'
}

async function onSave() {
  if (!canEdit.value || !dirty.value || saveStatus.value === 'saving') return
  if (!store.path) return
  saveStatus.value = 'saving'
  saveErrorMsg.value = ''
  try {
    const ok = await explorer.saveFile(store.path, content.value)
    if (ok) {
      originalContent.value = content.value
      saveStatus.value = 'saved'
      // 2s 后回归 idle
      setTimeout(() => {
        if (saveStatus.value === 'saved') saveStatus.value = 'idle'
      }, 2000)
    } else {
      saveStatus.value = 'error'
      saveErrorMsg.value = explorer.error || '未知错误'
    }
  } catch (e) {
    saveStatus.value = 'error'
    saveErrorMsg.value = e instanceof Error ? e.message : String(e)
    logger.error('FileViewerOverlay', '保存失败', e)
  }
}

function onSelectionChange(info: SelectionInfo | null) {
  if (!info) {
    selectionInfo.value = null
    selectionRect.value = null
    return
  }
  selectionInfo.value = info
  nextTick(() => computeSelectionRect())
}

// 计算选区在视口的位置（mirror-div 法），目前仅用于调试/扩展
function computeSelectionRect() {
  const editor: any = editorRef.value
  const textarea: HTMLTextAreaElement | null | undefined = editor?.textarea?.value ?? editor?.textarea
  if (!textarea || !selectionInfo.value) return
  const value = textarea.value
  const start = textarea.selectionStart
  const end = textarea.selectionEnd
  if (start === end) return

  const coords = getTextareaCoords(value, start, textarea)
  const coordsEnd = getTextareaCoords(value, end, textarea)
  if (!coords) return

  const taRect = textarea.getBoundingClientRect()
  const cs = getComputedStyle(textarea)
  const padTop = parseFloat(cs.paddingTop) || 0
  const padLeft = parseFloat(cs.paddingLeft) || 0

  const left = taRect.left + padLeft + coords.x
  const top = taRect.top + padTop + coords.y
  selectionRect.value = new DOMRect(left, top, Math.max(8, (coordsEnd?.x ?? coords.x) - coords.x), 0)
}

// 用 mirror div 计算 textarea 选区坐标（保留供后续扩展用）
function getTextareaCoords(
  value: string,
  pos: number,
  textarea: HTMLTextAreaElement
): { x: number; y: number; line: number } | null {
  try {
    const cs = getComputedStyle(textarea)
    const mirror = document.createElement('div')
    mirror.style.position = 'absolute'
    mirror.style.visibility = 'hidden'
    mirror.style.whiteSpace = 'pre-wrap'
    mirror.style.wordWrap = 'break-word'
    mirror.style.top = '0'
    mirror.style.left = '-9999px'
    mirror.style.width = `${textarea.clientWidth}px`
    const copyProps = [
      'fontFamily', 'fontSize', 'fontWeight', 'fontStyle',
      'letterSpacing', 'textTransform', 'wordSpacing',
      'textIndent', 'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft',
      'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth',
      'boxSizing', 'tabSize'
    ]
    for (const p of copyProps) {
      (mirror.style as any)[p] = cs.getPropertyValue(p.replace(/[A-Z]/g, (m) => '-' + m.toLowerCase()))
    }
    mirror.style.lineHeight = cs.lineHeight

    const before = value.substring(0, pos)
    const span = document.createElement('span')
    span.textContent = '\u200b'

    const lines = before.split('\n')
    const line = lines.length - 1
    const lastLine = lines[line]
    for (let i = 0; i < line; i++) {
      mirror.appendChild(document.createTextNode(lines[i]))
      mirror.appendChild(document.createElement('br'))
    }
    mirror.appendChild(document.createTextNode(lastLine))
    mirror.appendChild(span)

    document.body.appendChild(mirror)
    const spanRect = span.getBoundingClientRect()
    const mirrorRect = mirror.getBoundingClientRect()
    const result = {
      x: spanRect.left - mirrorRect.left,
      y: spanRect.top - mirrorRect.top,
      line
    }
    document.body.removeChild(mirror)
    return result
  } catch (e) {
    return null
  }
}

function onSendToAI() {
  if (!selectionInfo.value || !store.path) return

  // 没有活跃会话 → 兜底提示
  if (!sessionsStore.activeId) {
    alert('当前没有活跃的会话，无法发送。')
    return
  }

  const sel = selectionInfo.value

  // 1) 写入全局 AI 上下文（ModelChatPanel.send 时会通过 buildContextualMessage
  //    自动把 file/选区拼到消息里）。ChatContextBar 会读取这个 context
  //    渲染为顶部选择块（不再把摘要塞到输入框，避免纯文本污染）。
  setModelContext({
    filePath: store.path,
    fileContent: content.value,
    selectedText: sel.text,
    startLine: sel.startLine,
    endLine: sel.endLine,
    // 标记这个上下文来自哪个会话，ChatContextBar 用它检测"跨会话"风险
    sessionId: sessionsStore.activeId
  })

  // 2) 关闭覆盖层，让用户看到聊天窗口
  //    选区展示由 ChatContextBar 卡片负责，输入框保持空白，
  //    用户直接在这里敲问题即可。AI 收到消息时 buildContextualMessage
  //    会自动把 file/选区拼到消息里。
  handleClose(true)

  // 3) 通知 ModelChatPanel 聚焦输入框（但不写入任何摘要文本）
  requestFocusInput(sessionsStore.activeId)
}

function handleClose(skipDirtyConfirm = false) {
  if (!skipDirtyConfirm && dirty.value) {
    const ok = window.confirm('文件已修改但未保存，确定关闭？')
    if (!ok) return
  }
  // 注意：这里不要清空 AI 上下文。
  // onSendToAI 路径会先 setModelContext 再 handleClose(true)，
  // 关闭覆盖层后用户要继续在 ChatContextBar 卡片里看到选区。
  // 用户可点 ChatContextBar 上的 × 按钮或发送消息来清空上下文。
  store.close()
}

// Vue 模板事件 handler（不接受任何参数，强制走"询问保存"分支）
function onCloseRequest() {
  handleClose(false)
}

// 打开 / 路径变化时重新加载
watch(
  () => [store.isOpen, store.path],
  ([open, p]) => {
    if (open && p) {
      load()
    } else if (!open) {
      content.value = ''
      originalContent.value = ''
      loadError.value = null
      saveStatus.value = 'idle'
      viewMode.value = 'edit'
      selectionInfo.value = null
      selectionRect.value = null
    }
  },
  { immediate: true }
)

// 全局快捷键：Ctrl/Cmd+S
function onKeydown(e: KeyboardEvent) {
  if (!store.isOpen) return
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault()
    if (canEdit.value) onSave()
  }
}

window.addEventListener('keydown', onKeydown)
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  content.value = ''
  originalContent.value = ''
  loadError.value = null
  selectionInfo.value = null
  selectionRect.value = null
})
</script>

<style scoped>
.file-viewer-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  background: var(--color-bg);
  display: flex;
  flex-direction: column;
  animation: fadeIn 0.15s ease-out;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.overlay-header {
  display: grid;
  /* 三列：左 1fr / 中 auto / 右 1fr，
     中间列固定为内容宽度，左/右列吸收剩余空间，
     使 .header-center 视觉上真正水平居中 */
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  padding: 0.5rem 0.9rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
  gap: 0.75rem;
  min-height: 2.75rem;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
  justify-content: flex-start;
}

.file-icon { font-size: 1.1rem; flex-shrink: 0; }

.file-name {
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dirty-mark {
  color: #f59e0b;
  font-size: 0.7rem;
}

.header-center {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  justify-content: center;
  flex-shrink: 0;
}

.view-switch {
  display: flex;
  background: rgba(0, 0, 0, 0.06);
  border-radius: 0.3125rem;
  padding: var(--space-05);
}

.switch-btn {
  border: none;
  background: transparent;
  padding: 0.25rem 0.7rem;
  font-size: 0.78rem;
  border-radius: 0.25rem;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.12s;
}

.switch-btn.active {
  background: var(--color-bg);
  color: var(--color-text);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
}

.switch-btn:hover:not(.active) {
  color: var(--color-text);
}

.header-right {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  justify-content: flex-end;
  flex-shrink: 0;
}

.save-status {
  font-size: 0.72rem;
  color: var(--color-text-muted);
  padding: 0 0.2rem;
}

.save-status.saved { color: #10b981; }
.save-status.error { color: #ef4444; }

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  border: none;
  background: transparent;
  border-radius: 0.3125rem;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.12s;
}

.icon-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.08);
  color: var(--color-text);
}

.icon-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.save-btn:not(:disabled) {
  color: var(--color-primary);
}

.overlay-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  position: relative;
}

.editor-wrapper {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
}

/* 选中时浮出"发送到 AI" */
.send-to-ai {
  position: fixed;
  z-index: 9999;
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.4rem 0.8rem;
  border: none;
  border-radius: 1.125rem;
  background: var(--color-primary);
  color: #fff;
  font-size: 0.8rem;
  font-weight: 500;
  cursor: pointer;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.22);
  transition: all 0.12s;
}

.send-to-ai:hover {
  transform: translateY(-1px);
  box-shadow: 0 6px 16px rgba(0, 0, 0, 0.28);
}

.line-badge {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.7rem;
  padding: 0.05rem 0.4rem;
  background: rgba(255, 255, 255, 0.22);
  border-radius: 0.5rem;
}

.fade-pop-enter-active,
.fade-pop-leave-active {
  transition: opacity 0.12s, transform 0.12s;
}

.fade-pop-enter-from,
.fade-pop-leave-to {
  opacity: 0;
  transform: translateY(0.375rem);
}

/* --- Markdown 预览样式 --- */
.md-content {
  flex: 1;
  height: 100%;
  overflow: auto;
  padding: 1.2rem 1.8rem;
  max-width: 57.5rem;
  margin: 0 auto;
  font-size: 0.95rem;
  line-height: 1.7;
  color: var(--color-text);
}

.md-content :deep(h1),
.md-content :deep(h2),
.md-content :deep(h3) {
  margin: 0.9em 0 0.5em;
  font-weight: 600;
  line-height: 1.3;
}

.md-content :deep(h1) {
  font-size: 1.6rem;
  border-bottom: 1px solid var(--color-border);
  padding-bottom: 0.35em;
}

.md-content :deep(h2) { font-size: 1.3rem; }
.md-content :deep(h3) { font-size: 1.1rem; }

.md-content :deep(p) { margin: 0.5em 0; }

.md-content :deep(code) {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.88em;
  padding: 0.15em 0.4em;
  background: rgba(127, 127, 127, 0.12);
  border-radius: 0.1875rem;
}

.md-content :deep(pre) {
  margin: 0.7em 0;
  padding: 0.7em 0.9em;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 0.3125rem;
  overflow: auto;
}

.md-content :deep(pre code) {
  background: transparent;
  padding: 0;
  font-size: 0.85em;
}

.md-content :deep(ul),
.md-content :deep(ol) {
  padding-left: 1.6em;
  margin: 0.5em 0;
}

.md-content :deep(blockquote) {
  margin: 0.5em 0;
  padding: 0.3em 0.9em;
  border-left: 0.1875rem solid var(--color-border);
  color: var(--color-text-secondary);
}

.md-content :deep(a) {
  color: var(--color-primary);
  text-decoration: none;
}

.md-content :deep(a):hover { text-decoration: underline; }

.md-content :deep(table) {
  border-collapse: collapse;
  margin: 0.7em 0;
}

.md-content :deep(th),
.md-content :deep(td) {
  border: 1px solid var(--color-border);
  padding: 0.4em 0.7em;
}

.md-content :deep(th) { background: var(--color-surface); }

.binary-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  color: var(--color-text-muted);
  font-size: 0.9rem;
  gap: 0.4rem;
}

.binary-info { font-size: 0.78rem; opacity: 0.75; }

.loading-state,
.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  color: var(--color-text-muted);
  gap: 0.5rem;
  font-size: 0.9rem;
}

.error-state { color: #ef4444; }

.spinner {
  width: 1.5rem;
  height: 1.5rem;
  border: 0.1563rem solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }
</style>

