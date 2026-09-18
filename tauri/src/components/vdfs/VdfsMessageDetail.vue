<!--
  VdfsMessageDetail — VDFS `message` 渲染器（单条对话消息 = **转写列表的一项**）

  这是「转写即列表」在前端的落点：一条消息是一个普通节点，地址是
  `.vdfs/session/<sid>/消息/<mid>`；正文在**内容**里（`data`），结构在
  `attributes` 里（role / type / parent_id / seq / error / meta）。

  ## 为什么是只读的

  发言不是一次文件写入，而是一次**动作**（触发一整轮编排：模型调用 → 工具执行
  → 多轮流式落库）。后端对 `消息` 子树上的 `write` 显式拒绝（`Forbidden`），
  因此这里也不给保存 / 重命名 / 删除入口——**写入入口唯一**（聊天协议）。

  ## 与 `ext = session` 的分工

  - `session`（会话叶子）：聊天工作区，可发言；
  - `message`（列表项）：这条消息本身的只读视图，LLM 与前端读到的是同一份。

  正文随 `appended` 变更就地增长（见 `useVdfs.applyAppend`），因此流式期间
  这个视图不需要重读即可跟着长——这正是「流式即列表项的追加」的可视化。
-->
<template>
  <div class="vdfs-message">
    <header class="msg-head">
      <span class="role" :class="`role-${roleKey}`">{{ roleText }}</span>
      <span v-if="typeText" class="type">{{ typeText }}</span>
      <span class="status" :class="`status-${node.status}`">{{ statusText }}</span>
      <span v-if="seq !== null" class="seq">#{{ seq }}</span>
    </header>

    <p v-if="node.error" class="msg-error">{{ node.error }}</p>

    <pre v-if="body" class="msg-body">{{ body }}</pre>
    <p v-else class="msg-empty">（无正文：组合节点本身不承载内容）</p>

    <dl v-if="meta.length" class="msg-meta">
      <div v-for="row in meta" :key="row.key" class="meta-row">
        <dt>{{ row.key }}</dt>
        <dd>{{ row.value }}</dd>
      </div>
    </dl>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { VdfsFieldError, VdfsNode } from '@/schemas/vdfs'
import { MESSAGE_TYPE_TEXT } from '@/schemas/chat_message'
import { messageRoleLabel, messageStatusLabel, messageTypeLabel } from '@/registry/messageTypes'

// 渲染器统一契约（详见 VdfsTextDetail 同名说明）：
// `data` 是节点内容（消息正文），`node` 携带结构（attributes）
const props = defineProps<{
  node: VdfsNode
  data?: unknown
  error?: string
  fieldErrors?: VdfsFieldError[]
  saving?: boolean
}>()

defineEmits<{
  (e: 'save', payload: unknown): void
  (e: 'delete'): void
  (e: 'rename'): void
}>()

/** 正文（追加型变更就地拼进这个缓冲） */
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
.vdfs-message {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0.75rem 1rem;
  gap: 0.6rem;
}
.msg-head {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
  flex-shrink: 0;
}
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
