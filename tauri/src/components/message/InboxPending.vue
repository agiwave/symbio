<!--
  排队中的用户消息 ——「已入队、还没被消费」的统一呈现（T-077 步③）

  ## 它补的是哪一个窗口

  发言只是往会话收件箱写一条（`<会话地址>/inbox/<消息 id>`）。会话正忙时它**排队**，
  要等当前轮跑完才被取出、落库、发权威帧。落库后的显示由转写实时面负责
  （`sessionTranscriptSync`，**无论发送方是谁**都照实显示）；而排队这一段时间里，
  界面上什么都没有——连「前端自己发的那条」也只有 `working` 乐观置位（空白流 +
  一条等待骨架），「不是我发的消息」（CLI / telegram / 心跳 / 别的进程）更是一点
  痕迹都没有。本组件把那段时间显示出来。

  ## 为什么是「预览」而不是全文

  入队帧只带节点视图，正文是 `description` 里的**前 60 字**（后端 `inbox_item_node`）。
  排队气泡是状态提示，全文由它落库后的转写帧给——**同一个 id**，因此落地时是
  「换位置」（气泡消失、消息出现在流里），不是「多一条」。为全文再发一次 IPC
  是把一次入队从 1 帧变成 1 帧 + 2 请求，换来的东西几毫秒后自己会到。

  ## 外观为什么与用户气泡不同

  同一个区域里并排出现「已落库的用户消息」与「排队中的同一条消息」是可能的
  （转写帧到了、出队帧还没到，或反过来）。两者用同一副面孔就会读成两条消息，
  因此排队气泡用**虚线描边 + 降饱和** + 「待发送」标：它是**同一个事实的另一种
  状态**，视觉上必须能被区分。

  顺序即**消费顺序**：队列是 FIFO，`store` 的 `applyInboxItem` 保持到达顺序，
  这里按给定顺序渲染（`v-for` 不重排）。
-->
<template>
  <div v-if="entries.length > 0" class="inbox-pending" :aria-label="`${entries.length} 条消息排队中`">
    <!-- 一行说明：解释「为什么发了却还没出现在流里」。
         **只在不止一条时出现**：单条时气泡自带「待发送」标，再压一行字是噪音。
         （最常见的情形正是单条——前端自己刚发出去的那条。） -->
    <p v-if="entries.length > 1" class="inbox-pending-hint">
      {{ entries.length }} 条消息排队中 · 等待当前轮结束后按顺序处理
    </p>
    <div v-for="entry in entries" :key="entry.id" class="inbox-row">
      <div class="inbox-bubble" :title="entry.label">
        <span class="inbox-tag">待发送</span>
        <span class="inbox-text">{{ entry.label }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import type { InboxEntry } from '@/stores/sessionInboxSync'

defineProps<{
  /** 排队中的条目（顺序即消费顺序，调用方不得重排） */
  entries: InboxEntry[]
}>()
</script>

<style scoped>
.inbox-pending {
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.4rem);
}

.inbox-pending-hint {
  margin: 0;
  font-size: 0.72rem;
  color: var(--text-muted);
  text-align: right;
}

.inbox-row {
  display: flex;
  justify-content: flex-end;
}

/* 排队气泡：右对齐（与用户消息同一侧——它是同一条消息的前一个状态），
   虚线描边 + 透明底 = 「还没落」。不用新颜色令牌：颜色继承 muted，只由
   描边与底色表达状态。 */
.inbox-bubble {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  max-width: 80%;
  padding: 0.45rem 0.75rem;
  border: 1px dashed var(--border-default);
  border-radius: 0.875rem 0.875rem 0.25rem 0.875rem;
  background: transparent;
  color: var(--text-muted);
  font-size: 0.8rem;
  line-height: 1.5;
  word-break: break-word;
}

.inbox-tag {
  flex-shrink: 0;
  font-size: 0.68rem;
  padding: 0.02rem 0.35rem;
  border-radius: 62.4375rem;
  background: var(--surface-hover, rgba(128, 128, 128, 0.12));
  color: var(--text-secondary);
}

.inbox-text {
  min-width: 0;
  overflow: hidden;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
}
</style>
