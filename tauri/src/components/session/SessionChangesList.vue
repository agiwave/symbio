<!--
  SessionChangesList — 本会话「改过哪些文件」的只读清单（机制内置，唯一实现）

  消费 `session/changes` 下发的**呈现就绪行**，零业务知识：本组件不认识
  `vdfs_write` / `vdfs_edit`，也不判断哪些工具算写——动作名与文案全在 `label` 里。
  它只做两件事：按行排版，把点击原样上抛（目标地址由调用方经地址代数拼好）。

  ## 三种空必须分开说

  后端用 `has_wal`（事实源在不在）与 `readable`（读方能不能看）把空态判完了，
  本组件只按它们选一句话：

  - 没有事实源 ⇒ 这个会话还没跑过；
  - 有源但不可读 ⇒ 不给看（不是「没改过」）；
  - 有源、可读、清单为空 ⇒ 这一轮之前确实没动过工作目录。

  混成一句会把「权限不够」显示成「没有变更」——两者的处置完全不同。
-->
<template>
  <div class="changes-list">
    <p v-if="!changes.has_wal" class="changes-empty">这个会话还没有跑过一轮</p>
    <p v-else-if="!changes.readable" class="changes-empty">无权查看本会话的变更</p>
    <p v-else-if="changes.rows.length === 0" class="changes-empty">
      本会话还没有改动过工作目录
    </p>

    <ul v-else class="changes-rows">
      <li v-for="row in changes.rows" :key="row.address">
        <button
          type="button"
          class="change-row"
          :data-op="row.op"
          :title="row.path"
          @click="emit('open', row)"
        >
          <span class="change-path">{{ row.path }}</span>
          <span class="change-meta">
            <span class="change-label">{{ row.label }}</span>
            <span>第 {{ row.turn }} 轮</span>
            <span v-if="row.changes > 1">{{ row.changes }} 次</span>
          </span>
        </button>
      </li>
    </ul>
  </div>
</template>

<script setup lang="ts">
import type { ChangeRow, SessionChanges } from '@/services/sessionChanges'

defineProps<{
  /** 后端下发的清单（行的字段原样使用，本组件不换算、不翻译） */
  changes: SessionChanges
}>()

const emit = defineEmits<{
  /** 打开某一行对应的文件（地址由调用方拼好，本组件只带后端给的 `address`） */
  (e: 'open', row: ChangeRow): void
}>()
</script>

<style scoped>
.changes-list {
  display: flex;
  flex-direction: column;
}

.changes-empty {
  margin: 0;
  font-size: 0.75rem;
  color: var(--text-muted);
}

/* 行是整条可点区域：文件路径足够长，让用户瞄准图标或文字边缘点不划算 */
.changes-rows {
  list-style: none;
  margin: 0;
  padding: 0;
}

.changes-rows li + li {
  border-top: 1px solid var(--border-default);
}

.change-row {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  padding: 0.45rem 0.5rem;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.change-row:hover,
.change-row:focus-visible {
  background: var(--surface-hover);
}

.change-row:focus-visible {
  outline: 1px solid var(--accent);
  outline-offset: -1px;
}

.change-path {
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.75rem;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.change-meta {
  display: flex;
  gap: 0.5rem;
  font-size: 0.68rem;
  color: var(--text-muted);
}

/* 动作词是这一行「发生了什么」的主语，比轮号/次数略重 */
.change-label {
  color: var(--text-secondary);
}
</style>
