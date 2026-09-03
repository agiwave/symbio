<template>
  <aside class="session-explorer-panel">
    <header class="panel-header">
      <h3 class="panel-title">资源浏览器</h3>
      <div class="header-actions">
        <button class="icon-btn" @click="onRefresh" :disabled="!hasWorkdir" title="刷新">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="23 4 23 10 17 10" />
            <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
          </svg>
        </button>
      </div>
    </header>

    <div class="panel-body">
      <div v-if="!hasWorkdir" class="empty-explorer">
        <p class="empty-title">未绑定工作目录</p>
        <p class="empty-desc">选择工作目录后，资源树会显示在右侧</p>
      </div>
      <div v-else-if="error" class="error-state">
        <p>加载失败：{{ error }}</p>
        <button class="retry-btn" @click="onRefresh">重试</button>
      </div>
      <div v-else class="tree-scroll">
        <!-- 初次加载（还没有任何项）才显示 loading -->
        <div v-if="rootItems.length === 0 && loading" class="loading-state">加载中…</div>
        <div v-else-if="rootItems.length === 0" class="empty-explorer">
          <p>暂无文件</p>
          <p class="hint">目录可能为空</p>
        </div>
        <div v-else class="file-tree-content">
          <FileTreeNode
            v-for="item in rootItems"
            :key="item.path"
            :item="item"
            :level="0"
            :selected-path="selectedPath"
            :children="item.is_dir ? getChildren(item.path) : undefined"
            @select="onSelect"
          />
        </div>
      </div>
    </div>
  </aside>
</template>

<script setup lang="ts">
/**
 * 资源浏览器主面板（仅负责文件树）
 *
 * 文件预览改为全屏覆盖层（FileViewerOverlay），由 fileViewer store 管理。
 * 本组件只负责：选文件 → viewerStore.show(path, workdir)。
 *
 * 数据加载：
 * - 切 workdir / 切会话 → 自动 reloadExplorer
 * - 子目录懒加载 → 由 store.toggleExpand 内部处理；
 *   全局 loading 不再被污染，避免文件树 v-if 整体卸载
 */
import { computed, watch, onBeforeMount, onBeforeUnmount } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { useExplorerStore } from '@/stores/explorer'
import { useFileViewerStore } from '@/stores/fileViewer'
import { setGlobalWorkdir } from '@/services/plugin'
import FileTreeNode from '../FileTreeNode.vue'
import { logger } from '@/utils/logger'

const store = useSessionsStore()
const explorer = useExplorerStore()
const viewer = useFileViewerStore()

const hasWorkdir = computed(() => !!store.activeWorkdir)
const loading = computed(() => explorer.loading)
const error = computed(() => explorer.error)
const rootItems = computed(() => explorer.rootItems)
const selectedPath = computed(() => explorer.selectedPath)

function getChildren(path: string) {
  return explorer.getChildren(path)
}

async function onRefresh() {
  if (store.activeWorkdir) {
    setGlobalWorkdir(store.activeWorkdir)
  }
  await explorer.refresh()
}

// 点击节点：文件 → 打开全屏预览；目录 → 切换展开（store 内部处理）
function onSelect(path: string) {
  explorer.selectItem(path)
  const item = explorer.fileTree.get(path)
  if (item && !item.is_dir) {
    viewer.show(path, store.activeWorkdir ?? null)
  }
}

// ---- 数据加载逻辑（统一入口） ----
async function reloadExplorer() {
  // 切 workdir / session 时关掉可能还开着的预览
  viewer.close()

  const wd = store.activeWorkdir
  if (wd) {
    setGlobalWorkdir(wd)
    explorer.reset()
    try {
      await explorer.loadDirectory('')
    } catch (e) {
      logger.error('SessionExplorerPanel', '加载工作目录失败', e)
    }
  } else {
    // 没有 workdir：清空 explorer，确保"未绑定工作目录"状态下没有残留文件
    explorer.reset()
  }
}

// 关键 watcher：会话或 workdir 变化触发 reload（统一入口）
// - 同时覆盖「切换会话」和「同会话内改 workdir」两种场景
// - 不依赖 workdir 字符串是否变化：activeId 一变就 reload
// - wd=undefined 走 reset 分支，UI 显示"未绑定"
watch(
  [() => store.activeId, () => store.activeWorkdir],
  async ([id, wd]) => {
    viewer.close()
    if (!id) {
      explorer.reset()
      return
    }
    if (!wd) {
      explorer.reset()
      return
    }
    setGlobalWorkdir(wd)
    explorer.reset()
    try {
      await explorer.loadDirectory('')
    } catch (e) {
      logger.error('SessionExplorerPanel', '加载工作目录失败', e)
    }
  }
)

onBeforeMount(async () => {
  // 首次挂载：如果 active 会话已有 workdir，加载；否则只启动 watching
  if (store.activeId && store.activeWorkdir) {
    await reloadExplorer()
  }
  await explorer.startWatching()
})

onBeforeUnmount(() => {
  explorer.stopWatching()
})
</script>

<style scoped>
.session-explorer-panel {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  background: var(--color-bg);
  border-left: 1px solid var(--color-border);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.panel-title {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--color-text-secondary);
  margin: 0;
}

.header-actions {
  display: flex;
  gap: 0.25rem;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.5rem;
  height: 1.5rem;
  border: none;
  background: transparent;
  border-radius: 0.25rem;
  cursor: pointer;
  color: var(--color-text-secondary);
}

.icon-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.06);
  color: var(--color-text);
}

.icon-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.panel-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.tree-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0.25rem 0;
}

.file-tree-content {
  padding: 0;
}

.empty-explorer {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  padding: 1.5rem;
  text-align: center;
  color: var(--color-text-muted);
  font-size: 0.85rem;
}

.empty-explorer .empty-title {
  font-size: 0.9rem;
  color: var(--color-text-secondary);
  margin-bottom: 0.4rem;
}

.empty-explorer .empty-desc {
  font-size: 0.78rem;
  opacity: 0.7;
  line-height: 1.5;
}

.empty-explorer .hint {
  font-size: 0.75rem;
  opacity: 0.6;
}

.loading-state,
.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  padding: 1.5rem;
  font-size: 0.85rem;
  color: var(--color-text-muted);
}

.error-state { color: #ef4444; }

.retry-btn {
  margin-top: 0.5rem;
  padding: 0.3rem 0.7rem;
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: 0.25rem;
  cursor: pointer;
  font-size: 0.78rem;
}
</style>
