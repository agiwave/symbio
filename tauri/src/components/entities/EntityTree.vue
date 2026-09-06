<!--
  EntityTree — 树视图通用渲染件（机制内置，唯一实现）

  中栏两种结构形态之一（`ContainerKindInfo.view = "tree"`；另一种为列表）。
  机制只定义「层级 + 懒加载 + 选择」：
  - 节点 = 统一 EntitySummary：`id` = 容器内相对路径、`parent` = 父路径、
    `expandable` = 可展开提示；
  - 懒加载：展开可展开节点时经 `entities/list`（payload 携带 container +
    sub_kind + parent）拉取下一层，数据全部来自 provider 的 container 钩子；
  - 选择：emit select(id)，详情流沿用容器页既有机制（content 分流 /
    能力门控），本组件不感知任何场景语义（文件/目录等均为 provider 场景）；
  - 节点图标：entity 注册表按 kind 映射（provider 场景注册，如 "dir"）。
-->
<template>
  <div class="entity-tree" role="tree">
    <p v-if="error" class="tree-error">{{ error }}</p>
    <p v-else-if="!rootLoaded" class="tree-loading">加载中…</p>
    <p v-else-if="!childrenOf('').length" class="tree-empty">（空）</p>
    <EntityTreeNode
      v-for="node in childrenOf('')"
      :key="node.id"
      :node="node"
      :depth="0"
      :children-of="childrenOf"
      :is-expanded="isExpanded"
      :is-loading="isLoadingDir"
      :selected-id="selectedId"
      @toggle="toggle"
      @select="select"
    />
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { listEntities } from '@/services/entities'
import { logger } from '@/utils/logger'
import type { EntitySummary } from '@/schemas/entities'
import EntityTreeNode from './EntityTreeNode.vue'

const props = defineProps<{
  /** 容器所属 provider kind（entitiesOp 前缀解析用） */
  containerKind: string
  /** 容器条目 id */
  containerId: string
  /** 树视图子类别（声明 view = "tree" 的 container kind） */
  subKind: string
  /** 当前选中节点 id（容器页选中机制回显） */
  selectedId?: string | null
}>()

const emit = defineEmits<{ (e: 'select', id: string): void }>()

// === 节点状态：扁平表 + 层级派生（与懒加载协议对齐） ===
const nodes = ref(new Map<string, EntitySummary>())
const expanded = ref(new Set<string>())
const loadingDirs = ref(new Set<string>())
const loadedLevels = ref(new Set<string>())
const rootLoaded = ref(false)
const error = ref('')

/** 某父路径下已加载的直接子节点（可展开优先、名称字典序） */
function childrenOf(parentPath: string): EntitySummary[] {
  const out: EntitySummary[] = []
  for (const n of nodes.value.values()) {
    if ((n.parent ?? '') === parentPath) out.push(n)
  }
  out.sort((a, b) => {
    const da = a.expandable !== false
    const db = b.expandable !== false
    return da === db ? a.name.localeCompare(b.name) : da ? -1 : 1
  })
  return out
}

function isExpanded(path: string): boolean {
  return expanded.value.has(path)
}

function isLoadingDir(path: string): boolean {
  return loadingDirs.value.has(path)
}

/** 拉取一层子节点（parent = 相对路径，'' = 根层），合并进节点表 */
async function fetchLevel(parentPath: string) {
  if (loadingDirs.value.has(parentPath)) return
  error.value = ''
  loadingDirs.value.add(parentPath)
  try {
    const resp = await listEntities(props.containerKind, {
      container: props.containerId,
      subKind: props.subKind,
      parent: parentPath || undefined,
    })
    const next = new Map(nodes.value)
    // 该层旧节点先移除（刷新语义），再合并新节点
    for (const key of next.keys()) {
      if ((next.get(key)?.parent ?? '') === parentPath) next.delete(key)
    }
    for (const it of resp.items ?? []) next.set(it.id, it)
    nodes.value = next
  } catch (err) {
    logger.error('EntityTree', `加载 ${parentPath || '<root>'} 失败:`, err)
    error.value = `加载失败: ${err}`
  } finally {
    loadingDirs.value.delete(parentPath)
  }
}

/** 展开/收起：展开且该层未加载过 → 懒加载（返回为空则自然收敛为叶子） */
async function toggle(node: EntitySummary) {
  if (node.expandable === false) return
  const next = new Set(expanded.value)
  if (next.has(node.id)) {
    next.delete(node.id)
  } else {
    next.add(node.id)
    if (!loadedLevels.value.has(node.id)) {
      loadedLevels.value.add(node.id)
      void fetchLevel(node.id)
    }
  }
  expanded.value = next
}

function select(node: EntitySummary) {
  emit('select', node.id)
}

// 根层加载：挂载 / 目标容器或子类别变化时重置
async function loadRoot() {
  nodes.value = new Map()
  expanded.value = new Set()
  loadingDirs.value = new Set()
  loadedLevels.value = new Set()
  rootLoaded.value = false
  await fetchLevel('')
  rootLoaded.value = true
}

onMounted(loadRoot)
watch(() => [props.containerKind, props.containerId, props.subKind], loadRoot)
</script>

<style scoped>
.entity-tree {
  height: 100%;
  overflow-y: auto;
  padding: 0.5rem;
  font-size: 0.85rem;
}
.tree-loading,
.tree-empty,
.tree-error {
  margin: 0;
  padding: 1rem 0.5rem;
  text-align: center;
  color: var(--text-muted);
  font-size: 0.85rem;
}
.tree-error { color: var(--danger-fg); }
</style>
