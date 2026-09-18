/**
 * 消息渲染器装配 —— 「渲染器标识 → 组件」的唯一装配点（UI 资产，非数据契约）
 *
 * 机制分层（与 `vdfsTypes.ts` / `vdfsRenderers.ts` 同一套）：
 * - `schemas/chat_message.ts`   数据契约（取值词表 + 形状，零呈现知识）
 * - `registry/messageTypes.ts`  纯 UI 映射：facets → 渲染器标识；标识 → 组件登记表
 * - 本文件                      把标识绑定到具体 Vue 组件（**唯一 import 组件的地方**）
 *
 * 这样 `messageTypes` 可被服务层 / 组合式安全引用而不牵连组件图；新增一种消息类型
 * 只需在此登记一行（未登记的渲染器由视图兜底，会话流永不空白）。
 *
 * ## 兜底为什么复用 `TextNode`
 *
 * 未登记类型（后端先行上线了新 `type`、前端还是旧版）落到 `fallback`。
 * `TextNode` 的末支本就是「把 `content` 当正文渲染」，正是这种场景想要的：
 * 用户看到内容而不是空白或报错。与 `vdfsRenderers` 把 `dir` / `fallback`
 * 都指向只读详情是同一思路。
 */

import { markRaw } from 'vue'
import TurnGroupNode from '@/components/message/TurnGroupNode.vue'
import TextNode from '@/components/message/TextNode.vue'
import ToolCallNode from '@/components/message/ToolCallNode.vue'
import UserPromptNode from '@/components/message/UserPromptNode.vue'
import CompressionNode from '@/components/message/CompressionNode.vue'
import { registerMessageRenderer } from './messageTypes'

// 机制级呈现形态（与场景无关）
registerMessageRenderer('turn', markRaw(TurnGroupNode))
registerMessageRenderer('text', markRaw(TextNode))
registerMessageRenderer('tool_call', markRaw(ToolCallNode))
registerMessageRenderer('user_prompt', markRaw(UserPromptNode))
registerMessageRenderer('compression', markRaw(CompressionNode))
// 未登记类型的兜底：仍把内容当正文显示（会话流永不空白）
registerMessageRenderer('fallback', markRaw(TextNode))
