<!--
  ChatComposer — 会话输入区的**唯一装配**（输入框 + 选项行）

  输入区在任何地方都是这两件一起出现，且顺序、叠加关系固定：

    ChatInputArea   文本 / 图片 / 发送键
    ChatOptionBar   级联选项行（工作目录 / 智能体 / 模型 / 模式 / 风险等级…）

  此前这两件在 `Session.vue`（新建草稿态）与 `ModelChatPanel.vue`（已落盘会话）
  各装配一次，连同各自的 ref 与 `resetHeight()` 调用点也是两份。装配一旦有两处，
  「输入区由什么组成」就有两个答案——加一件（如引用选择器）必须记得改两处。
  这里收成一处：`ChatInputArea + ChatOptionBar` = 输入区。

  ## 两种模式只差一个 `sessionId`

  - **有 `sessionId`**（已落盘会话）：选项行的写入直达后端（`vdfs/write` 会话
    `metadata`），值由调用方经 `values` 传入（节点 `attributes.metadata`）；
  - **无 `sessionId`**（新建草稿态）：选项行的选择缓冲在机制内部，随「创建会话」
    一并作为 metadata 补丁写入（`getDraftMetadata()` 取用）。

  两种模式的差别对输入框不存在，故本组件不为此分叉：只把 `sessionId` / `definition`
  / `values` / `scope` 原样透传下去，缓冲语义由 `ChatOptionBar` /
  `useSessionOptionBar` 自己按「有没有会话」决定。

  定义与值都不在这里回读：**定义随节点下发**，谁手上有那个会话的节点，谁负责传
  （见 `definition` prop 的说明）。

  ## 暴露面

  父级需要的能力**只经这一个 ref**（此前是 `draftInputRef` + `draftOptionsRef`
  两个 ref 各管一半）：
  - `resetHeight()` —— 发送后输入框高度复位；
  - `focus()`       —— 主动聚焦；
  - `getDraftMetadata()` —— 草稿态选项缓冲（创建会话时取用）。

  ## 与「首条消息交接」的关系

  本组件只负责**呈现与取值**，不参与懒创建握手，也不认识 `pendingFirstMessage`。
  交接（创建后把首条消息交给新会话）由 store 的邮箱 + 新会话面板消费，见
  `stores/sessions.createSessionWithFirstMessage` 与 `ModelChatPanel`。
-->
<template>
  <div class="chat-composer">
    <ChatInputArea
      ref="inputRef"
      v-model="text"
      v-model:attached-images="attachedImages"
      :is-loading="isLoading"
      :autofocus="autofocus"
      @submit="$emit('submit')"
    />
    <ChatOptionBar
      ref="optionsRef"
      :session-id="sessionId"
      :definition="definition"
      :values="values"
      :scope="scope"
    />
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import ChatInputArea from './ChatInputArea.vue'
import ChatOptionBar from './ChatOptionBar.vue'
import type { ImageAttachment } from '@/types'
import type { DetailDefinition } from '@/schemas/vdfs'

withDefaults(
  defineProps<{
    /** 发送 / 停止在途（决定发送键形态与可点性） */
    isLoading?: boolean
    /** 当前会话 id；缺省 = 草稿态（选项缓冲于机制内部，随创建写入） */
    sessionId?: string
    /**
     * 选项定义（会话节点 `schema` / `new_type.schema`）。
     *
     * 定义**随节点下发**，故由调用方透传而不是在本组件里回读：会话详情页拿的是
     * `props.node.schema`，与会话树解耦的面板（`ModelChatPanel`）拿的是 store 会话
     * 清单里那一项的 `schema`——两处都是「它手上那个会话的节点」。
     */
    definition?: DetailDefinition | null
    /** 字段当前值（节点 `attributes.metadata`）；缺省 = 全按定义缺省值显示 */
    values?: Record<string, unknown> | null
    /** 条件求值的额外键（节点 `attributes`，如 `message_count`） */
    scope?: Record<string, unknown> | null
    /** 挂载即聚焦输入框 */
    autofocus?: boolean
  }>(),
  {
    isLoading: false,
    sessionId: undefined,
    definition: null,
    values: null,
    scope: null,
    autofocus: false,
  }
)

defineEmits<{ (e: 'submit'): void }>()

const text = defineModel<string>({ default: '' })
const attachedImages = defineModel<ImageAttachment[]>('attachedImages', { default: () => [] })

const inputRef = ref<{
  resetHeight: () => void
  textarea: HTMLTextAreaElement | null
} | null>(null)
const optionsRef = ref<{ getDraftMetadata: () => Record<string, unknown> } | null>(null)

/** 输入框自身暴露的能力原样透传（父级只需认这一个 ref） */
defineExpose({
  resetHeight: () => inputRef.value?.resetHeight(),
  focus: () => inputRef.value?.textarea?.focus(),
  getDraftMetadata: () => optionsRef.value?.getDraftMetadata() ?? {},
})
</script>

<style scoped>
/* 纵向堆叠：输入框在上、选项行在下（选项行自带 margin-top，不在此重复间距） */
.chat-composer {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
</style>
