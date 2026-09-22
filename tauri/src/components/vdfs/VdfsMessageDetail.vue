<!--
  VdfsMessageDetail — VDFS `message` 渲染器（单条对话消息 = **转写列表的一项**）

  这是「转写即列表」在前端的落点：一条消息是一个普通节点，地址是
  `<根>/session/<sid>/消息/<mid>`；正文在**内容**里（`data`），结构在
  `attributes` 里（role / type / parent_id / seq / error / meta）。

  ## 为什么是只读的

  发言不是一次文件写入，而是一次**动作**（触发一整轮编排：模型调用 → 工具执行
  → 多轮流式落库）。后端对 `消息` 子树上的 `write` 显式拒绝（`Forbidden`），
  因此这里也不给保存 / 重命名 / 删除入口——**写入入口唯一**（聊天协议）。
  机制动作集也因此不会注入任何东西：消息节点的访问位只有 `r`（无 `w`）。

  ## 与 `ext = session` 的分工

  - `session`（会话叶子）：聊天工作区，可发言；
  - `message`（列表项）：这条消息本身的只读视图，LLM 与前端读到的是同一份。

  正文来自一次 `vdfs/read`。**本视图不随流式增长**：实时面是转写流
  （`session/stream`），由 `ext = session` 的聊天工作区消费；这里是同一份数据的
  **只读视角**，重选（或该节点发生 VDFS 变更）即重读。

  为什么不给消息节点发 VDFS 变更让它跟着长：`kind = "vdfs"` 是一条**独立的无序
  通道**，在它上面捎带正文快照，一次迟到的帧就会把正文回退——正是批次 E 从会话
  运行态上拆掉的那类问题。真要做，也该让本视图直接消费转写流，而不是往这条通道
  加载荷（见 `schemas/vdfs.VdfsChange` 的「两个字段就是全部」）。
-->
<template>
  <DetailShell :mechanism-actions="mechanismActions" :mechanism-busy="mechanismBusy" :error="error">
    <!-- 头部即「角色 / 类型 / 状态 / 序号」四枚小标，没有标题文本 -->
    <template #title>
      <span class="role" :class="`role-${roleKey}`">{{ roleText }}</span>
      <span v-if="typeText" class="type">{{ typeText }}</span>
      <span class="status" :class="`status-${node.status}`">{{ statusText }}</span>
      <span v-if="seq !== null" class="seq">#{{ seq }}</span>
    </template>

    <p v-if="node.error" class="msg-error">{{ node.error }}</p>

    <pre v-if="body" class="msg-body">{{ body }}</pre>
    <p v-else class="msg-empty">（无正文：组合节点本身不承载内容）</p>

    <dl v-if="meta.length" class="msg-meta">
      <div v-for="row in meta" :key="row.key" class="meta-row">
        <dt>{{ row.key }}</dt>
        <dd>{{ row.value }}</dd>
      </div>
    </dl>
  </DetailShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import DetailShell from './DetailShell.vue'
import type { VdfsRendererProps } from './rendererContract'
import { MESSAGE_TYPE_TEXT } from '@/schemas/chat_message'
import { messageRoleLabel, messageStatusLabel, messageTypeLabel } from '@/registry/messageTypes'

const props = defineProps<VdfsRendererProps>()

defineEmits<{
  (e: 'save', payload: unknown): void
  (e: 'delete'): void
  (e: 'rename'): void
}>()

/** 正文（由 `useVdfs` 的一次 `vdfs/read` 填充；本视图不做增量拼接） */
const body = computed(() => (typeof props.data === 'string' ? props.data : ''))

/** `attributes.role`（VDFS 只透传，渲染器自行取用） */
const roleKey = computed(() => String(props.node.role ?? 'unknown'))

// 文案一律查 `registry/messageTypes`——那是消息词表的**唯一来源**。
// 本组件不再自带 ROLE_TEXT / TYPE_TEXT / STATUS_TEXT 副本（曾经是三份实现之一）。
const roleText = computed(() => messageRoleLabel(roleKey.value))

/** 类型缺省不显示（文本是常态，标出来只是噪音） */
const typeText = computed(() => {
  const t = props.node.type
  if (!t || t === MESSAGE_TYPE_TEXT) return ''
  return messageTypeLabel(String(t))
})

const statusText = computed(() =>
  props.node.status ? messageStatusLabel(String(props.node.status)) : '未知',
)

const seq = computed(() => {
  const s = props.node.seq
  return typeof s === 'number' ? s : null
})

/** 结构补充信息（正文已在上面，不重复展示 meta 里的正文副本） */
const meta = computed(() => {
  const out: Array<{ key: string; value: string }> = []
  if (props.node.parent_id) out.push({ key: '父节点', value: String(props.node.parent_id) })
  const m = props.node.meta
  if (m && typeof m === 'object' && Object.keys(m).length) {
    out.push({ key: 'meta', value: JSON.stringify(m) })
  }
  return out
})
</script>

<style scoped>
.role {
  font-size: 0.7rem;
  font-weight: var(--font-weight-semibold);
  padding: 0.1rem 0.5rem;
  border-radius: var(--radius-full);
  background: var(--accent-subtle-bg);
  color: var(--accent);
}

.role-user {
  background: var(--success-subtle-bg);
}

.role-tool {
  background: var(--surface-sunken);
  color: var(--text-secondary);
}

.type,
.seq {
  font-size: 0.68rem;
  color: var(--text-muted);
}

.status {
  font-size: 0.68rem;
  color: var(--text-muted);
}

.status-streaming {
  color: var(--accent);
}

.status-failed {
  color: var(--danger-fg);
}

.msg-error {
  margin: 0;
  font-size: 0.8rem;
  color: var(--danger-fg);
}

.msg-body {
  margin: 0;
  font-family: inherit;
  font-size: 0.82rem;
  line-height: 1.6;
  color: var(--text-primary);
  white-space: pre-wrap;
  word-break: break-word;
}

.msg-empty {
  margin: 0;
  font-size: 0.78rem;
  color: var(--text-muted);
}

.msg-meta {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.meta-row {
  display: flex;
  gap: 0.75rem;
  font-size: 0.75rem;
}

.meta-row dt {
  width: 5rem;
  flex-shrink: 0;
  color: var(--text-muted);
}

.meta-row dd {
  margin: 0;
  color: var(--text-secondary);
  word-break: break-all;
}
</style>
