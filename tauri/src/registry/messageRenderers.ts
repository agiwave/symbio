/**
 * 消息渲染器装配 —— 「渲染器标识 → 组件」的唯一装配点（UI 资产，非数据契约）
 *
 * 机制分层（与 `vdfsTypes.ts` / `vdfsRenderers.ts` 同一套）：
 * - `schemas/chat_node.ts`   数据契约（取值词表 + 形状，零呈现知识）
 * - `registry/messageTypes.ts`  纯 UI 映射：facets → 渲染器标识；标识 → 组件登记表
 * - 本文件                      把标识绑定到具体 Vue 组件（**唯一 import 组件的地方**）
 *
 * B2a：渲染器标识就是 wire 五值 `NodeKind`（`typing` 占位走 `fallback`）——
 * 五个组件、五格形状，前端分派轴不再有任何业务名（`turn` / `tool_call` / `reasoning` /
 * `compression` / `memory` / `user_prompt` 全部并入形状格）。
 *
 * ## 兜底为什么复用 `BodyNode`
 *
 * 未登记类型（后端先行上线了新 `type`、前端还是旧版）落到 `fallback`。
 * `BodyNode` 的末支本就是「把 `content` 当正文渲染」，正是这种场景想要的：
 * 用户看到内容而不是空白或报错。与 `vdfsRenderers` 把 `dir` / `fallback`
 * 都指向只读详情是同一思路。
 */

import { markRaw } from 'vue'
import SkeletonNode from '@/components/message/SkeletonNode.vue'
import BodyNode from '@/components/message/BodyNode.vue'
import NoticeNode from '@/components/message/NoticeNode.vue'
import InteractionNode from '@/components/message/InteractionNode.vue'
import { registerMessageRenderer } from './messageTypes'

// 机制级形状（与场景无关）——一格一个渲染器
registerMessageRenderer('skeleton', markRaw(SkeletonNode))
registerMessageRenderer('body', markRaw(BodyNode))
registerMessageRenderer('notice', markRaw(NoticeNode))
registerMessageRenderer('interaction', markRaw(InteractionNode))
// 未登记类型 / typing 占位的兜底：仍把内容当正文显示（会话流永不空白）
registerMessageRenderer('fallback', markRaw(BodyNode))
