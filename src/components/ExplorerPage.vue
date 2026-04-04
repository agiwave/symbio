<template>
  <div class="explorer-page" :class="{ 'chat-visible': chatVisible }">
    <!-- 左侧：文件树 -->
    <aside class="file-tree">
      <div class="file-tree-header">
        <h3>资源浏览器</h3>
        <div class="file-tree-actions">
          <button class="icon-btn" @click="refresh" title="刷新">⟳</button>
        </div>
      </div>
      <div class="file-tree-content">
        <div v-if="loading" class="loading-state">
          加载中...
        </div>
        <div v-else-if="error" class="error-state">
          <p>错误：{{ error }}</p>
        </div>
        <div v-else-if="rootItems.length === 0 && !loading" class="empty-state">
          <p>暂无文件</p>
          <p class="hint">工作区可能为空</p>
        </div>
        <FileTreeNode
          v-for="item in rootItems"
          :key="item.path"
          :item="item"
          :level="0"
          :selected-path="selectedPath"
          :children="item.is_dir ? getChildren(item.path) : undefined"
          @select="selectItem"
          @expand="expandDirectory"
        />
      </div>
    </aside>

    <!-- 右侧：内容区 -->
    <main class="content-area" ref="contentAreaRef">
      <div v-if="selectedPath" class="content-container">
        <header class="content-header">
          <div class="path-breadcrumb">
            <span class="path-text">{{ selectedPath || '/' }}</span>
          </div>
          <div class="content-actions">
            <button
              v-if="hasUnsavedChanges"
              class="action-btn save-btn"
              :class="{ 'saving': isSaving }"
              @click="handleSave"
              :title="isSaving ? '保存中...' : '保存修改 (Ctrl+S)'"
            >
              {{ isSaving ? '⏳' : '💾' }}
            </button>
            <span v-if="hasUnsavedChanges" class="unsaved-indicator">●</span>
            <button
              class="action-btn"
              :class="{ active: chatVisible }"
              @click="chatVisible = !chatVisible"
              title="AI 对话"
            >
              💬
            </button>
          </div>
        </header>
        <div class="content-body">
          <!-- 目录：显示子项列表 -->
          <div v-if="selectedItem?.is_dir" class="dir-view">
            <div class="dir-header">
              <span class="dir-name">名称</span>
              <span class="dir-size">大小</span>
            </div>
            <div class="dir-list">
              <div
                v-for="child in childrenItems"
                :key="child.path"
                class="dir-item"
                :class="{ 'is-dir': child.is_dir }"
                @click="selectItem(child.path)"
              >
                <span class="item-icon">{{ child.is_dir ? '📁' : getFileIcon(child.name) }}</span>
                <span class="item-name">{{ child.name }}</span>
                <span class="item-size">{{ formatSize(child.size) }}</span>
              </div>
            </div>
          </div>

          <!-- 文件：显示内容预览 -->
          <div v-else-if="!isFileLoading && fileContent" class="file-preview">
            <!-- Markdown 文件使用 MarkdownEditor -->
            <MarkdownEditor
              v-if="isMarkdownFile"
              :key="selectedPath"
              :model-value="editorContent"
              :file-path="selectedPath || undefined"
              class="md-editor"
              @content-change="onContentChange"
              @request-save="handleSave"
            />
            <!-- 其他文本文件使用通用代码编辑器 -->
            <CodeEditor
              v-else-if="isTextFile"
              :key="selectedPath"
              :model-value="editorContent"
              :file-path="selectedPath || undefined"
              class="code-editor-wrapper"
              @content-change="onContentChange"
              @request-save="handleSave"
              @selection-change="handleCodeEditorSelection"
            />
            <!-- 无法预览的二进制文件 -->
            <pre v-else class="code-block"><code>{{ fileContent }}</code></pre>
          </div>

          <!-- 文件：加载中 -->
          <div v-else-if="isFileLoading" class="loading-file">
            <div class="spinner"></div>
            <p>正在加载文件内容...</p>
          </div>

          <!-- 文件：未加载或不可读 -->
          <div v-else class="empty-file">
            <p>文件内容无法预览或尚未加载</p>
          </div>
        </div>
      </div>
      <div v-else class="empty-content">
        <p>选择一个文件或目录</p>
        <p class="hint">左侧浏览工作区中的文件</p>
      </div>
    </main>

    <!-- AI 对话侧边栏 (可拉出/隐藏) -->
    <aside class="chat-sidebar" v-show="chatVisible" :style="{ width: chatWidth + 'px' }">
      <div class="chat-header">
        <h3>AI 对话</h3>
        <button class="close-btn" @click="chatVisible = false" title="隐藏">×</button>
      </div>
      <div class="chat-content">
        <AIChatPanel
          :session-id="EXPLORER_SESSION_ID"
          :messages="chatMessages"
          :on-update-messages="updateChatMessages"
        />
      </div>
    </aside>

    <!-- 拖动手柄 -->
    <div 
      v-show="chatVisible" 
      class="chat-resize-handle" 
      :class="{ 'dragging': isResizing }"
      @mousedown="startResize"
    ></div>
    
    <!-- AI 选区对话框 -->
    <AISelectionDialog :state="aiSelection" />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useExplorerStore, type FileItem } from '../stores/explorer'
import FileTreeNode from './FileTreeNode.vue'
import AIChatPanel from './AIChatPanel.vue'
import AISelectionDialog from './AISelectionDialog.vue'
import MarkdownEditor from './MarkdownEditor.vue'
import CodeEditor from './CodeEditor.vue'
import { type SessionMessage } from '../services/session'
import { useAISelection } from '@/composables/useAISelection'
import { onDirChanged, onFileChanged, startWatching, stopWatching } from '../services/watcher'
import { setAIContext } from '@/composables/useAIContext'

const store = useExplorerStore()

// UI 状态
const chatVisible = ref(false)
const contentAreaRef = ref<HTMLElement | null>(null)
const chatWidth = ref(320) // 默认宽度
const isResizing = ref(false)
const startWidth = ref(0)
const startX = ref(0)

// 编辑状态
const editorContent = ref<string>('')
const hasUnsavedChanges = ref(false)
const isSaving = ref(false)
const originalContent = ref<string>('')
const isFileLoading = ref(false)

// AI 选区交互 - 使用 composable
const aiSelection = useAISelection({ sessionId: 'explorer-selection-ai' })

// AI 对话状态 - 使用固定的 session_id
const EXPLORER_SESSION_ID = 'explorer-ai-session'
const chatMessages = ref<SessionMessage[]>([])

function updateChatMessages(messages: SessionMessage[]) {
  chatMessages.value = messages
}

// 文件树状态
const fileTree = computed(() => store.fileTree)
const rootItems = computed(() => store.rootItems)
const selectedPath = computed(() => store.selectedPath)
const fileContent = computed(() => store.fileContent)
const loading = computed(() => store.loading)
const error = computed(() => store.error)

// 当前选中的项
const selectedItem = computed(() => {
  if (!selectedPath.value) return null
  return fileTree.value.get(selectedPath.value) || null
})

// 当前选中目录的子项
const childrenItems = computed(() => {
  if (!selectedPath.value) return []
  return store.getChildren(selectedPath.value)
})

// 判断是否是 Markdown 文件
const isMarkdownFile = computed(() => {
  if (!selectedPath.value) return false
  return selectedPath.value.toLowerCase().endsWith('.md')
})

// 判断是否是文本文件（可编辑）
const isTextFile = computed(() => {
  if (!selectedPath.value) return false
  const ext = selectedPath.value.toLowerCase().split('.').pop()
  const textExts = [
    'txt', 'json', 'xml', 'yaml', 'yml', 'html', 'css', 'scss',
    'js', 'ts', 'jsx', 'tsx', 'vue', 'py', 'rs', 'go', 'java',
    'c', 'cpp', 'h', 'hpp', 'sh', 'bash', 'sql', 'graphql',
    'toml', 'ini', 'cfg', 'conf', 'env', 'gitignore', 'dockerfile',
    'php', 'rb', 'swift', 'kt', 'scala', 'r', 'm', 'mdx',
  ]
  return ext ? textExts.includes(ext) : false
})

onMounted(async () => {
  store.init()
  // 添加键盘事件监听
  document.addEventListener('keydown', handleKeydown)
  // 添加选区监听
  if (contentAreaRef.value) {
    contentAreaRef.value.addEventListener('mouseup', handleMouseUp)
  }
  // 添加离开页面提醒
  window.addEventListener('beforeunload', handleBeforeUnload)

  // 启动文件监听
  try {
    await startWatching()
    
    // 监听目录变化
    unlistenDir = await onDirChanged((event) => {
      console.log('[Explorer] Dir changed:', event.path, event.kind)
      refresh()
    })
    
    // 监听文件变化
    unlistenFile = await onFileChanged((event) => {
      console.log('[Explorer] File changed:', event.path, event.kind)
      // 如果正在手动保存，跳过文件监听器重载，避免覆盖内容
      if (store.isSavingFile) {
        console.log('[Explorer] Skipping reload during save')
        return
      }
      // 如果当前打开的文件被修改，重新加载
      if (selectedPath.value && event.path.endsWith(selectedPath.value)) {
        console.log('[Explorer] Reloading current file:', selectedPath.value)
        store.selectItem(selectedPath.value)
      }
      // 刷新文件树
      refresh()
    })
  } catch (err) {
    console.error('[Explorer] Failed to start watching:', err)
  }
})

let unlistenFile: (() => void) | null = null
let unlistenDir: (() => void) | null = null

onUnmounted(async () => {
  document.removeEventListener('keydown', handleKeydown)
  if (contentAreaRef.value) {
    contentAreaRef.value.removeEventListener('mouseup', handleMouseUp)
  }
  window.removeEventListener('beforeunload', handleBeforeUnload)

  // 清理拖动事件
  document.removeEventListener('mousemove', doResize)
  document.removeEventListener('mouseup', stopResize)

  // 停止文件监听
  unlistenFile?.()
  unlistenDir?.()
  try {
    await stopWatching()
  } catch (err) {
    console.error('[Explorer] Failed to stop watching:', err)
  }
})

// 键盘事件处理
function handleKeydown(e: KeyboardEvent) {
  // Escape 关闭 AI 选区对话框
  if (aiSelection.handleEscape(e)) return
  // Ctrl+K 打开 AI 选区对话框(带上当前文件上下文)
  if (aiSelection.handleCtrlK(e, { 
    filePath: selectedPath.value || undefined, 
    fileContent: editorContent.value || undefined 
  })) return
  // Ctrl+S 保存文件
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault()
    handleSave()
  }
}

// 选区事件处理
function handleMouseUp(e: MouseEvent) {
  aiSelection.handleMouseUp(e, contentAreaRef.value || undefined, {
    filePath: selectedPath.value || undefined,
    fullContent: fileContent.value || undefined
  })
}

// CodeEditor 选区事件处理
function handleCodeEditorSelection(data: { text: string; startLine: number; endLine: number } | null) {
  if (data) {
    // 获取 textarea 元素的位置用于对话框定位
    const codeEditorEl = contentAreaRef.value?.querySelector('.code-textarea') as HTMLTextAreaElement | null
    
    if (aiSelection.visible.value) {
      // 对话框已打开，更新选区
      aiSelection.selectedText.value = data.text
      aiSelection.savedSelection.value = {
        text: data.text,
        rect: codeEditorEl?.getBoundingClientRect() || { left: 0, top: 0, width: 0, height: 0 } as DOMRect,
        startLine: data.startLine,
        endLine: data.endLine,
        filePath: selectedPath.value || undefined,
        fullContent: fileContent.value || undefined,
      }
    } else {
      // 打开对话框
      aiSelection.openForSelection(data.text, codeEditorEl?.getBoundingClientRect() || { left: 0, top: 0, width: 0, height: 0 } as DOMRect, {
        startLine: data.startLine,
        endLine: data.endLine,
        filePath: selectedPath.value || undefined,
        fullContent: fileContent.value || undefined,
      })
    }
  } else {
    // 无选区，关闭对话框
    if (aiSelection.visible.value) {
      aiSelection.close()
    }
  }
}

// 拖动调整宽度
function startResize(e: MouseEvent) {
  e.preventDefault()
  isResizing.value = true
  startWidth.value = chatWidth.value
  startX.value = e.clientX

  document.addEventListener('mousemove', doResize)
  document.addEventListener('mouseup', stopResize)
}

function doResize(e: MouseEvent) {
  if (!isResizing.value) return
  
  const delta = startX.value - e.clientX
  const newWidth = startWidth.value + delta
  
  // 限制最小和最大宽度
  const minWidth = 280
  const maxWidth = window.innerWidth * 0.6
  
  chatWidth.value = Math.max(minWidth, Math.min(maxWidth, newWidth))
}

function stopResize() {
  isResizing.value = false
  document.removeEventListener('mousemove', doResize)
  document.removeEventListener('mouseup', stopResize)
}

// 离开页面提醒
function handleBeforeUnload(e: BeforeUnloadEvent) {
  if (hasUnsavedChanges.value) {
    e.preventDefault()
    e.returnValue = '有未保存的修改，确定要离开吗？'
    return '有未保存的修改，确定要离开吗？'
  }
}

// 刷新
function refresh() {
  store.refresh()
}

// 选择项
async function selectItem(path: string) {
  // 如果有未保存的修改，提示用户
  if (hasUnsavedChanges.value) {
    const shouldSave = confirm('当前文件有未保存的修改，是否先保存？')
    if (shouldSave) {
      await handleSave()
    }
  }

  // 设置加载状态
  isFileLoading.value = true
  
  // 重置编辑状态
  hasUnsavedChanges.value = false
  editorContent.value = ''
  originalContent.value = ''

  try {
    // 等待文件内容加载完成
    await store.selectItem(path)

    // 文件加载完成后，一次性设置编辑器内容
    editorContent.value = store.fileContent || ''
    originalContent.value = store.fileContent || ''
    hasUnsavedChanges.value = false

    // 更新全局 AI 上下文
    setAIContext({
      filePath: path,
      fileContent: store.fileContent || undefined,
      selectedText: undefined,
      startLine: undefined,
      endLine: undefined,
    })
  } finally {
    // 无论成功还是失败，都清除加载状态
    isFileLoading.value = false
  }
}

// 内容变化
function onContentChange(value: string) {
  editorContent.value = value
  // 只有当内容与原始内容不同时才标记为有未保存修改
  hasUnsavedChanges.value = value !== originalContent.value
}

// 保存文件
async function handleSave() {
  if (!selectedPath.value || !hasUnsavedChanges.value || isSaving.value) return

  isSaving.value = true
  try {
    const success = await store.saveFile(selectedPath.value, editorContent.value)
    if (success) {
      originalContent.value = editorContent.value
      hasUnsavedChanges.value = false
    }
  } catch (err) {
    console.error('[Explorer] Save failed:', err)
  } finally {
    isSaving.value = false
  }
}

// 展开目录
async function expandDirectory(path: string) {
  await store.expandDirectory(path)
}

// 获取子项
function getChildren(parentPath: string): FileItem[] {
  return store.getChildren(parentPath)
}

// 格式化大小
function formatSize(size?: number): string {
  if (size === undefined) return '-'
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  if (size < 1024 * 1024 * 1024) return `${(size / (1024 * 1024)).toFixed(1)} MB`
  return `${(size / (1024 * 1024 * 1024)).toFixed(1)} GB`
}

// 获取文件图标
function getFileIcon(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase()
  const icons: Record<string, string> = {
    'md': '📝',
    'txt': '📄',
    'js': '📜',
    'ts': '📘',
    'vue': '💚',
    'json': '📋',
    'yaml': '📝',
    'yml': '📝',
    'html': '🌐',
    'css': '🎨',
    'png': '🖼️',
    'jpg': '🖼️',
    'jpeg': '🖼️',
    'gif': '🖼️',
    'svg': '🖼️',
    'pdf': '📕',
    'zip': '📦',
    'tar': '📦',
    'gz': '📦',
    'rs': '🦀',
    'py': '🐍',
    'go': '🔹',
    'java': '☕',
    'c': '⚙️',
    'cpp': '⚙️',
    'h': '⚙️',
    'hpp': '⚙️',
    'sh': '📜',
    'bash': '📜',
  }
  return icons[ext || ''] || '📄'
}
</script>

<style scoped>
.explorer-page {
  display: flex;
  height: 100%;
  width: 100%;
}

/* 文件树 */
.file-tree {
  width: var(--panel-width, 280px);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.file-tree-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.file-tree-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--color-text-secondary);
}

.file-tree-actions {
  display: flex;
  gap: 0.5rem;
}

.icon-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  border-radius: 4px;
  cursor: pointer;
  font-size: 1rem;
  line-height: 1;
  transition: all 0.2s;
}

.icon-btn:hover {
  background: #f0f0f0;
  color: var(--color-text);
}

.file-tree-content {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
}

.loading-state,
.empty-state,
.error-state {
  text-align: center;
  padding: 2rem 1rem;
  color: var(--color-text-muted);
}

.error-state {
  color: #dc2626;
}

/* 内容区 */
.content-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.content-container {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
}

.content-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.path-breadcrumb {
  flex: 1;
  font-size: 0.875rem;
  color: var(--color-text-secondary);
}

.path-text {
  font-family: 'Fira Code', 'Consolas', monospace;
  background: #f5f5f5;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
}

.content-actions {
  display: flex;
  gap: 0.5rem;
}

.action-btn {
  width: 32px;
  height: 32px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 1rem;
  transition: background 0.2s;
}

.action-btn:hover,
.action-btn.active {
  background: #f0f0f0;
}

.save-btn {
  position: relative;
}

.save-btn.saving {
  cursor: not-allowed;
  opacity: 0.6;
}

.unsaved-indicator {
  color: #f59e0b;
  font-size: 1.2rem;
  line-height: 1;
  margin-right: 0.25rem;
  animation: pulse 2s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.content-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

/* 代码编辑器容器 */
.code-editor-wrapper {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  overflow: hidden;
}

/* 目录视图 */
.dir-view {
  padding: 0.5rem;
  flex: 1;
  display: flex;
  flex-direction: column;
}

.dir-header {
  display: flex;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--color-border);
  font-size: 0.75rem;
  color: var(--color-text-muted);
  text-transform: uppercase;
}

.dir-name {
  flex: 1;
}

.dir-size {
  width: 80px;
  text-align: right;
}

.dir-list {
  display: flex;
  flex-direction: column;
  flex: 1;
  overflow-y: auto;
}

.dir-item {
  display: flex;
  align-items: center;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.2s;
  gap: 0.5rem;
}

.dir-item:hover {
  background: #f0f0f0;
}

.dir-item.is-dir {
  font-weight: 500;
}

.item-icon {
  font-size: 1rem;
  width: 20px;
  text-align: center;
}

.item-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item-size {
  width: 80px;
  text-align: right;
  font-size: 0.75rem;
  color: var(--color-text-muted);
}

/* 文件预览 */
.file-preview {
  flex: 1;
  padding: 1rem;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.md-editor {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  overflow: hidden;
}

.code-block {
  flex: 1;
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 1rem;
  border-radius: 8px;
  overflow: auto;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.875rem;
  line-height: 1.5;
  margin: 0;
  min-height: 0;
}

.code-block code {
  white-space: pre;
}

.empty-file,
.empty-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
  text-align: center;
  min-height: 100%;
}

.loading-file {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
  gap: 1rem;
}

.loading-file .spinner {
  width: 40px;
  height: 40px;
  border: 3px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.empty-content .hint {
  font-size: 0.875rem;
  margin-top: 0.5rem;
  opacity: 0.7;
}

/* AI 对话侧边栏 */
.chat-sidebar {
  width: 320px;
  background: var(--color-surface);
  border-left: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.chat-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.chat-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 1.25rem;
  color: var(--color-text-muted);
  border-radius: 4px;
}

.close-btn:hover {
  background: #f0f0f0;
}

.chat-history {
  flex: 1;
  overflow-y: auto;
  padding: 1rem;
}

.chat-content {
  flex: 1;
  overflow: hidden;
}

/* 拖动手柄样式 */
.chat-resize-handle {
  width: 6px;
  cursor: col-resize;
  background: transparent;
  flex-shrink: 0;
  position: relative;
  z-index: 1500; /* 提高层级,避免被其他元素遮挡 */
}

.chat-resize-handle:hover,
.chat-resize-handle.dragging {
  background: var(--color-primary);
  opacity: 0.3;
}
</style>
