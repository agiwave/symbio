<!--
  子节点递归渲染 —— **会话流里唯一知道「怎么往下递归」的地方**

  会话流是一棵无限递归的树（Turn → 思考/正文/工具 → 工具的过程/结果 → …），
  每层都要把 `depth + 1` 传下去、把 `retry/delete/edit` 原样抛上来。
  这套契约只写一次，个体渲染器只说「这里有一批子节点」，
  不再各自重复 `:depth="(depth ?? 0) + 1"` 与三行事件转发。

  ## 循环引用说明

  本组件 import `MessageNode`，而 `MessageNode` 经渲染器注册表间接 import 本组件，
  形成环。这是**「类型 → 组件」注册表 + 递归渲染**必然产生的结构，ESM 能正确处理：
  环上唯一的引用点在渲染函数内（`_createVNode`），模块求值期不触碰该绑定，
  因此求值顺序不会读到未初始化的导出。
-->
<template>
  <MessageNode
    v-for="child in nodes"
    :key="child.id"
    :node="child"
    :depth="childDepth"
    :parent-type="parentType"
    :parent-failed="parentFailed"
    @retry="emit('retry', $event)"
    @delete="emit('delete', $event)"
    @edit="emit('edit', $event)"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ChatMessage } from '@/schemas/chat_message'
import MessageNode from '../MessageNode.vue'

const props = defineProps<{
  /** 待渲染的子节点（顺序即渲染顺序，由 store 按 `seq` 排好） */
  nodes: ChatMessage[]
  /** **父节点**的深度；子节点实际深度为 `depth + 1` */
  depth: number
  /** 子节点应看到的父节点类型（`turn` / `tool_call`）——决定「工具返回」「正文直排」等判定 */
  parentType?: string
  /** 父节点（ToolCall）是否已失败：子节点据此隐藏自身错误框，避免重复报错 */
  parentFailed?: boolean
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const childDepth = computed(() => (props.depth ?? 0) + 1)
</script>
