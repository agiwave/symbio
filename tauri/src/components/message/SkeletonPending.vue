<!--
  等待骨架 ——「这一轮已经开始，但还没有任何内容」的统一呈现

  ## 为什么抽成组件

  它的触发条件有两个来源，而**观感必须一致**（同节奏三点、同文案位置）：

  1. `SkeletonNode`：根级骨架节点已到达、尚无子节点 → 挂在骨架容器里（成员语义明确）；
  2. `ModelChatPanel`：**会话节点说「在跑」，但流里没有任何在途节点** → 挂在流末尾
     兜底。这一条是「骨架节点的变更丢了 / 还没到」时的呈现保险：状态是**会话
     节点**的属性，节点没到不等于会话没在跑（见 `session/docs/018-node-state-streaming.md`
     §4.0「变更不重放」）。缺了它，用户点完发送会看到「什么都没有发生」。

  两处各写一份三点动画，就会出现「一处改了另一处没改」的静默分叉——节奏不一致
  不会报错、不会被测试拦住，只是看起来不对。因此这里只有一份实现。

  调用方约束：**同一时刻只应出现一个**。`ModelChatPanel` 只在「没有任何在途节点」
  时兜底（那时骨架等待态必然不显示），两者因此天然互斥，不会同时出现两条。
-->
<template>
  <div class="skeleton-pending">
    <span class="skeleton-pending-dots"><span /><span /><span /></span>
    <span class="skeleton-pending-text">{{ text }}</span>
  </div>
</template>

<script setup lang="ts">
withDefaults(
  defineProps<{
    /** 等待文案（会话级兜底用「正在思考…」，骨架容器里带智能体名） */
    text?: string
  }>(),
  { text: '正在思考…' },
)
</script>

<style scoped>
.skeleton-pending {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.1rem;
}
.skeleton-pending-dots {
  display: inline-flex;
  gap: var(--space-1);
}
.skeleton-pending-dots span {
  width: 0.375rem;
  height: 0.375rem;
  border-radius: 50%;
  background: var(--accent);
  animation: skeleton-pulse 1.2s infinite ease-in-out;
}
.skeleton-pending-dots span:nth-child(2) {
  animation-delay: 0.2s;
}
.skeleton-pending-dots span:nth-child(3) {
  animation-delay: 0.4s;
}
@keyframes skeleton-pulse {
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
.skeleton-pending-text {
  font-size: 0.8rem;
  color: var(--text-muted);
}
</style>
