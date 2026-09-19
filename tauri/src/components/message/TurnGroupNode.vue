<!--
  Turn 渲染器 —— 一轮响应的**透明分组容器**（根级助手回合与子会话回合共用）

  三种形态（对齐 Claude / Codex 类智能体的会话流）：
    ① 等待骨架：Turn 已创建但无任何子节点且运行中 → 三点脉动 +「{智能体} 正在思考…」；
    ② 子节点直排：思考/正文/工具出现后容器完全隐藏（无头部、无外框），子节点直接纵排；
    ③ 组级交代条：Turn 失败或中止 → 交代条 +「重试」入口
       （resume action=retry_turn：删除响应子树 → 重新走 LLM 请求）。

  子会话 Turn（role=tool）走同一形态（分形复用）：工具「过程」段以与主会话一致的
  「思考 / 正文 / 工具」缩进节点直排呈现；**组级操作（删除/重试）仅归属根级助手 Turn**——
  子会话 Turn 的节点是工具过程段的临时广播（不落父会话存储），
  retry_turn / delete 缺少后端锚点，故不提供。
-->
<template>
  <div class="msg turn-group" :class="[statusClass]">
    <span v-if="isRootTurn" class="turn-actions" @click.stop>
      <button class="node-act" title="删除" @click.stop="emit('delete', node.id)">🗑</button>
    </span>

    <!-- ① 等待态 -->
    <div v-if="isPending" class="turn-pending">
      <span class="turn-pending-dots"><span /><span /><span /></span>
      <span class="turn-pending-text">{{ agentName }} 正在思考…</span>
    </div>

    <!-- ② 子节点直排 + ③ 组级交代条 -->
    <template v-else>
      <MessageChildren
        :nodes="children"
        :depth="depth ?? 0"
        parent-type="turn"
        @retry="emit('retry', $event)"
        @delete="emit('delete', $event)"
        @edit="emit('edit', $event)"
      />
      <!-- `failed`（出错了）与 `aborted`（用户终止）都渲染交代条。
           中止不是故障，因此文案与图标都与失败区分开，但**重试入口照给**。 -->
      <MessageErrorBox
        v-if="isUnsettled"
        :text="errorText"
        :aborted="isAborted"
        :retryable="isRootTurn"
        variant="turn"
        @retry="emit('retry', node.id)"
      />
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import {
  MESSAGE_TYPE_TURN,
  isUnsettledMessageStatus,
  type ChatMessage,
} from '@/schemas/chat_message'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  agentNameOf,
  isAbortedStatus,
  isStreamingStatus,
  type MessageFacets,
} from '@/registry/messageTypes'
import MessageChildren from './MessageChildren.vue'
import MessageErrorBox from './MessageErrorBox.vue'

const props = defineProps<{
  node: ChatMessage
  facets: MessageFacets
  depth?: number
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const children = computed(() => props.node.children || [])
const statusClass = computed(() => `status-${props.facets.status}`)
const agentName = computed(() => agentNameOf(props.node, props.facets.subSession))

/** 根级助手 Turn：唯一承载组级操作（悬停删除 / 组级重试）的 Turn */
const isRootTurn = computed(
  () => props.facets.type === MESSAGE_TYPE_TURN && !props.facets.subSession,
)
/** 等待态：Turn 已广播但尚无任何子节点且仍在运行 → 显示「正在思考…」骨架 */
const isPending = computed(
  () => isStreamingStatus(props.facets.status) && children.value.length === 0,
)
/** 需要向用户交代的非正常终态（失败 / 中止） */
const isUnsettled = computed(() => isUnsettledMessageStatus(props.facets.status))
const isAborted = computed(() => isAbortedStatus(props.facets.status))

const { errorText } = useMessageContent(
  () => props.node,
  () => props.facets,
)
</script>

<style scoped>
/* ── Turn 响应分组（透明容器）──
   容器本身无任何视觉框：子节点直排，仅作为等待骨架与组级交代条的挂载点。 */
.msg.turn-group {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
}
/* 悬停删除入口：默认隐藏，悬停整组时右上角浮现 */
.msg.turn-group .turn-actions {
  position: absolute;
  top: -0.2rem;
  right: 0;
  display: none;
  z-index: 1;
}
.msg.turn-group:hover .turn-actions {
  display: inline-flex;
}
/* 微按钮：与 `NodeShell` 头部操作同一视觉原子。
   两处各自持有样式块（scoped 无法跨组件共享），改一处需同步另一处。 */
/* 等待骨架：三点脉动 +「正在思考…」 */
.turn-pending {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.1rem;
}
.turn-pending-dots {
  display: inline-flex;
  gap: var(--space-1);
}
.turn-pending-dots span {
  width: 0.375rem;
  height: 0.375rem;
  border-radius: 50%;
  background: var(--accent);
  animation: turn-pulse 1.2s infinite ease-in-out;
}
.turn-pending-dots span:nth-child(2) {
  animation-delay: 0.2s;
}
.turn-pending-dots span:nth-child(3) {
  animation-delay: 0.4s;
}
@keyframes turn-pulse {
  0%,
  80%,
  100% {
    opacity: 0.25;
    transform: scale(0.85);
  }
  40% {
    opacity: 1;
    transform: scale(1);
  }
}
.turn-pending-text {
  font-size: 0.8rem;
  color: var(--text-muted);
}
</style>
