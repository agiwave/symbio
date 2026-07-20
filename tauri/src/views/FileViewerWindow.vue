<template>
  <div class="file-viewer-window">
    <header v-if="fileInfo" class="file-viewer-header">
      <h2 class="file-title">{{ fileInfo.title || basename(fileInfo.path) }}</h2>
      <span class="file-path" :title="fileInfo.path">{{ fileInfo.path }}</span>
    </header>

    <main class="file-viewer-body" v-if="fileInfo">
      <div v-if="loading" class="loading-state">
        <div class="spinner"></div>
        <p>加载文件中…</p>
      </div>

      <div v-else-if="loadError" class="error-state">
        <p>加载失败：{{ loadError }}</p>
      </div>

      <template v-else>
        <!-- Markdown 文件 -->
        <MarkdownEditor
          v-if="isMarkdown"
          :key="fileInfo.path"
          :model-value="content"
          :file-path="fileInfo.path"
          class="md-editor"
          readonly
        />
        <!-- 文本代码文件 -->
        <CodeEditor
          v-else-if="isText"
          :key="fileInfo.path"
          :model-value="content"
          :file-path="fileInfo.path"
          class="code-editor-wrapper"
          readonly
        />
        <!-- 二进制或其他 -->
        <pre v-else class="raw-block"><code>{{ content }}</code></pre>
      </template>
    </main>

    <main v-else class="file-viewer-body no-file">
      <p class="no-file-text">未选择文件</p>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, onBeforeRouteUpdate } from 'vue-router'
import { listen } from '@tauri-apps/api/event'
import { callPlugin } from '@/services/plugin'
import MarkdownEditor from '@/components/MarkdownEditor.vue'
import CodeEditor from '@/components/CodeEditor.vue'

interface FileInfo {
  path: string
  workdir?: string
  title?: string
}

const route = useRoute()
const fileInfo = ref<FileInfo | null>(null)
const content = ref('')
const loading = ref(false)
const loadError = ref<string | null>(null)
let unlisten: (() => void) | null = null

const isMarkdown = computed(() => /\.(md|markdown)$/i.test(fileInfo.value?.path || ''))
const isText = computed(() => {
  const p = fileInfo.value?.path || ''
  return /\.(txt|json|ya?ml|toml|js|ts|tsx|jsx|vue|rs|py|java|go|c|cpp|h|cs|rb|php|sh|bash|sql|html|css|scss|less|xml|ini|log|env|conf|cfg)$/i.test(p)
})

function basename(p: string): string {
  if (!p) return ''
  return p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
}

async function loadFile(info: FileInfo) {
  fileInfo.value = info
  loading.value = true
  loadError.value = null
  content.value = ''
  try {
    // 调用 explorer/read 读取文件内容
    const result: any = await callPlugin('explorer/read', {
      path: info.path,
      workdir: info.workdir
    })
    // 后端可能返回 {content: '...'} 或 { data: '...'} 或直接字符串
    if (typeof result === 'string') {
      content.value = result
    } else if (result?.content != null) {
      content.value = result.content
    } else if (result?.data != null) {
      content.value = result.data
    } else if (result != null) {
      content.value = JSON.stringify(result, null, 2)
    }
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

function parseFromUrl() {
  const q = route.query
  const path = (q.path as string) || ''
  if (!path) {
    fileInfo.value = null
    return
  }
  const info: FileInfo = {
    path,
    workdir: (q.workdir as string) || undefined,
    title: (q.title as string) || undefined
  }
  loadFile(info)
}

onMounted(async () => {
  parseFromUrl()
  // 监听来自主窗口的"切换文件"事件
  unlisten = await listen<{ path: string; workdir?: string; title?: string }>('file-viewer:load-file', (e) => {
    if (e.payload?.path) {
      loadFile(e.payload)
    }
  })
})

onBeforeRouteUpdate(() => {
  parseFromUrl()
})

onBeforeUnmount(() => {
  unlisten?.()
})
</script>

<style scoped>
.file-viewer-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  width: 100vw;
  background: var(--color-bg);
  overflow: hidden;
}

.file-viewer-header {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  padding: 0.6rem 1rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.file-title {
  font-size: 0.95rem;
  font-weight: 500;
  color: var(--color-text);
  margin: 0;
}

.file-path {
  font-size: 0.75rem;
  color: var(--color-text-muted);
  word-break: break-all;
}

.file-viewer-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  position: relative;
}

.md-editor,
.code-editor-wrapper {
  width: 100%;
  height: 100%;
}

.raw-block {
  width: 100%;
  height: 100%;
  margin: 0;
  padding: 1rem;
  overflow: auto;
  font-family: monospace;
  font-size: 0.8rem;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--color-surface);
  color: var(--color-text);
}

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
}

.spinner {
  width: 24px;
  height: 24px;
  border: 3px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.no-file {
  align-items: center;
  justify-content: center;
}

.no-file-text {
  color: var(--color-text-muted);
  font-size: 0.9rem;
}
</style>
