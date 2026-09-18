<!--
  对话消息节点 —— **分派器**（把节点路由到它的渲染器）

  ## 本组件只做三件事

  1. 把 props 摊成 `facets`（`facetsOf`，与个体渲染器**同一份实现**，
     因此两侧对同一节点的判定必然一致）；
  2. 用 `facets` 解析渲染器标识（`messageRendererKey`）并取出组件（`resolveMessageRenderer`）；
  3. 把事件原样抛上去（`retry` / `delete` / `edit`）。

  **不含任何「哪个类型怎么画」的判定**——那是 `registry/messageTypes.ts` 的事。
  新增一种消息类型的代价是「词表加一个取值 + 映射表加一行 + 装配点登记一行」，
  不需要改本文件，也不需要改任何渲染组件里的 if 链。

  ## 渲染路由

  | 渲染器        | 接管                                  | 组件                              |
  |--------------|---------------------------------------|-----------------------------------|
  | `turn`       | 轮次响应分组（根级 + 子会话共用）        | `message/TurnGroupNode.vue`       |
  | `text`       | 用户气泡 / 思考 / 正文 / 工具请求 / 返回 | `message/TextNode.vue`            |
  | `tool_call`  | 工具调用三段式卡片                     | `message/ToolCallNode.vue`        |
  | `user_prompt`| 待用户响应（提问 / 工具确认）           | `message/UserPromptNode.vue`      |
  | `compression`| 上下文压缩（系统动作）                  | `message/CompressionNode.vue`     |
  | `fallback`   | 未登记类型（后端先行上线的新 type）      | `message/TextNode.vue`            |

  ## 递归与父子契约

  渲染树是**无限递归**的（Turn → 思考/正文/工具 → 工具的过程/结果 → …）。
  向下递归的唯一入口是 `message/MessageChildren.vue`：它统一施加 `depth + 1`
  与 `parentType`，因此「层级缩进」「工具返回识别」「正文直排」这些依赖父上下文的
  判定只有一个来源。

  - `depth`：0 = 主会话根级；≥2 才施加缩进与引导竖线（depth=1 与主回合左缘齐平）；
  - `parentType`：只用于算 `facets`（`toolResult` / `responseText`），不再往下透传；
  - `parentFailed`：父 ToolCall 已失败时，工具结果子节点不再重复渲染错误框。

  ## 设计要点（详见各渲染器文件头）

  - **Turn 响应分组**三形态：等待骨架 / 子节点直排（容器完全隐藏）/ 组级交代条；
  - **统一折叠式节点**：可点击头部（图标 + 标题 + 状态标签）+ 折叠体，折叠策略集中
    在 `registry/messageTypes.messageDefaultOpen`；
  - **失败重试两级分派**（由 ModelChatPanel 按 msg.type 路由）：
    tool_call 失败 → resume `retry`（原参数重执行工具）；
    其余失败（Turn 及其叶子）→ resume `retry_turn`（删除响应子树 → 重新 LLM 请求）；
  - **失败终态只信服务端**（persist_failure 广播 Failed Update），前端不做启发式标记；
  - 层级只靠「缩进 + 左侧引导竖线」表达，纵向间隔全由单一变量 `--msg-gap` 驱动。
-->
<template>
  <component
    :is="renderer"
    :node="node"
    :facets="facets"
    :depth="depth"
    :parent-failed="parentFailed"
    @retry="emit('retry', $event)"
    @delete="emit('delete', $event)"
    @edit="emit('edit', $event)"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ChatMessage } from '@/schemas/chat_message'
// 副作用导入：装配点把「标识 → 组件」登记进 messageTypes 的注册表。
// 分派器因此**不需要知道任何具体组件**（新增渲染器不必改这里）。
import '@/registry/messageRenderers'
import { facetsOf, resolveMessageRenderer, type MessageFacets } from '@/registry/messageTypes'

const props = defineProps<{
  node: ChatMessage
  /** 嵌套深度：0 = 主会话根级；>0 = 工具 / 子 agent 内部的子会话流层级 */
  depth?: number
  /** 父节点类型：参与 `facets` 推导（工具返回 / 正文直排），不再往下透传 */
  parentType?: string
  /** 父节点（ToolCall）是否已失败：工具结果子节点据此隐藏自身错误框，
   *  避免与父 ToolCall 的错误框重复——错误只由"造成中止的节点"呈现一次。 */
  parentFailed?: boolean
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

/** 呈现判定入参：**唯一**的「props → facets」实现，个体渲染器直接消费它 */
const facets = computed<MessageFacets>(() => facetsOf(props.node, props.parentType))
const renderer = computed(() => resolveMessageRenderer(facets.value))
</script>
