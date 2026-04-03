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
          <div v-else-if="fileContent" class="file-preview">
            <!-- Markdown 文件使用 MarkdownEditor -->
            <MarkdownEditor
              v-if="isMarkdownFile"
              :key="selectedPath"
              :model-value="fileContent"
              class="md-editor"
            />
            <!-- 其他文件使用代码块预览 -->
            <pre v-else class="code-block"><code>{{ fileContent }}</code></pre>
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
    <aside class="chat-sidebar" v-show="chatVisible">
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
    
    <!-- AI 选区对话框 -->
    <AISelectionDialog :state="aiSelection" />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useExplorerStore, type FileItem } from '../stores/explorer'
import FileTreeNode from './FileTreeNode.vue'
import AIChatPanel from './AIChatPanel.vue'
import AISelectionDialog from './AISelectionDialog.vue'
import MarkdownEditor from './MarkdownEditor.vue'
import { createSessionId, type SessionMessage } from '../services/session'
import { useAISelection } from '@/composables/useAISelection'
import { onDirChanged, onFileChanged, startWatching, stopWatching, type BrowserFileChangeEvent } from '../services/watcher'

const store = useExplorerStore()

// UI 状态
const chatVisible = ref(false)
const contentAreaRef = ref<HTMLElement | null>(null)

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
const currentPath = computed(() => store.currentPath)
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

onMounted(async () => {
  store.init()
  // 添加键盘事件监听
  document.addEventListener('keydown', handleKeydown)
  // 添加选区监听
  if (contentAreaRef.value) {
    contentAreaRef.value.addEventListener('mouseup', handleMouseUp)
  }
  
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
      // 如果当前打开的文件被修改，重新加载
      if (selectedPath.value && event.path.includes(selectedPath.value)) {
        if (selectedPath.value) {
          store.selectItem(selectedPath.value)
        }
      }
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
  // Ctrl+K 打开 AI 选区对话框
  if (aiSelection.handleCtrlK(e)) return
}

// 选区事件处理
function handleMouseUp(e: MouseEvent) {
  aiSelection.handleMouseUp(e, contentAreaRef.value || undefined)
}

// 刷新
function refresh() {
  store.refresh()
}

// 选择项
async function selectItem(path: string) {
  await store.selectItem(path)
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

.content-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
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
</style>
