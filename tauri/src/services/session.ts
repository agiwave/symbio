/**
 * Session 服务 —— 会话域的**纯 VDFS 门面**
 *
 * ## 这里已经没有任何会话专用路由
 *
 * 本文件曾有一张 `SESSION_ROUTES` 表（消息的清空 / 删除 / 改写三条）与一个
 * `callSession` 信封函数。它们连同后端的三个 `invoke_*` 与三个 schema 一并退役
 * （2026-09-18）——**同一个操作两份实现**正是要消灭的东西。
 *
 * | 操作 | 入口 |
 * |---|---|
 * | 列会话清单 | `vdfs/list(<根>/session)` |
 * | 读整份转写 | `vdfs/read(<根>/session/<id>)` |
 * | 新建会话 | `vdfs/write(<根>/session, { create: true })` |
 * | 删除会话 | `vdfs/delete(<根>/session/<id>)` |
 * | 改 metadata / 标题 | `vdfs/write(<根>/session/<id>)` |
 * | **改写某条消息** | `vdfs/write(<根>/session/<id>/message/<mid>)` |
 * | **删除某条及其后** | `vdfs/action(…/message/<mid>, "truncate")` |
 * | **清空历史** | `vdfs/action(…/message, "clear")` |
 *
 * 本文件保留的只是**地址拼接 + 形状适配**（把 VDFS 域响应映射成 store 习惯的
 * 形状），没有任何协议知识——新增一种会话操作**不需要**在这里加路由。
 *
 * ## 地址里的「哪个空间」是**参数**，不是常量
 *
 * 上表里的 `<根>/session` 只是**默认**挂载目录。子智能体空间
 * （`<根>/agent/<id>/session`）是一棵完整子树、内部有自己的 `session` 挂载，
 * 因此每个函数都收一个可选的 `mountDir`：省略 = 默认（根空间，行为与从前一致），
 * 传入即作用于那个空间。**没有第二份实现**——同一个函数体按参数拼地址。
 *
 * ## 发言为什么不在上表
 *
 * 发言不是写入也不是动作，是**一轮编排**（模型调用 → 工具执行 → 流式落库）。
 * 但它的**入口**同样是地址：往 `<A>/inbox/<iid>` 写一条即入队（ADR-026），
 * 由空间自己在空闲时消费。见 `composables/useChatConnection`。
 */

import { deleteVdfs, listVdfs, readVdfs, runVdfsAction, writeVdfs } from './vdfs'
import { READBACK_REASON } from './readback'
import { ensureSessionMountDir, ensureSessionScheme } from './vdfsScheme'
import {
  VDFS_ACTION_CLEAR,
  VDFS_ACTION_TRUNCATE,
  VDFS_EXT_SESSION,
  vdfsExtOf,
  vdfsMessageAddr,
  vdfsMessagesAddr,
  vdfsSessionAddr,
} from '@/schemas/vdfs'
import { ChatMessage as SessionMessage } from '../schemas/chat_message'
import * as SessionList from '../schemas/session_list'
import type { SessionMetadata } from '../schemas/session_meta'

export type { SessionMessage }

export type { SessionListItem } from '../schemas/session_list'
export type { SessionMetadata } from '../schemas/session_meta'

/**
 * 获取会话列表（VDFS：`<根>/session` 的目录内容）
 *
 * 会话挂载点已把清单所需字段挂在节点上（`message_count` / `metadata` /
 * `meta_tags`，VDFS 只透传场景字段），故这里把节点直接映射为
 * `SessionListItem`，以便既有 `sessions` store / 对话组件保持兼容。
 *
 * `vdfs/list` 是会话清单的**唯一**读入口：不存在与之并行的第二套清单协议。
 *
 * 这里按 `ext` 过滤——**只认会话节点**，不按名字特判（文件名可改，语义不变）。
 * 后端已经不再把插件配置文件（`PLUGIN.yml`，`ext = form`）并列进会话清单
 * （「清单 = 业务列表」；它进设置菜单走的是 ConfigurableVisitor 那条通道），
 * 过滤因此是**防御**：万一哪天清单里混进了非会话节点，会话列表也不会被污染。
 */
/**
 * 列会话清单。
 *
 * 不传 `limit` 时请求里**不带**窗口参数——后端行为与从前逐字节一致
 * （有界列表是可选能力，不是新契约）。
 *
 * `mountDir` 省略 = **默认挂载目录**（根空间那份，`ensureSessionMountDir()`），
 * 与从前逐字节一致；子智能体空间传 `agent/<id>/session`。
 */
export async function listSessions(
  limit?: number,
  mountDir?: string
): Promise<SessionList.SessionListItem[]> {
  // 不传 limit 时**单参调用**——请求形状必须与从前一致（多一个 undefined
  // 实参也会被 `toHaveBeenCalledWith` 认成「多传了一个参数」）
  const path = mountDir ?? (await ensureSessionMountDir())
  const resp =
    limit === undefined
      ? await listVdfs(READBACK_REASON.LIST_REFRESH, path)
      : await listVdfs(READBACK_REASON.LIST_REFRESH, path, { limit })
  return (resp.items || [])
    .filter((n) => vdfsExtOf(n) === VDFS_EXT_SESSION)
    .map((n) => {
      const v = n as Record<string, any>
      return {
        id: n.name,
        // 后端 display_title：metadata.title 优先，否则从会话内容自动生成
        name: n.title ?? '',
        message_count: Number(v.message_count ?? 0),
        updated_at: n.updated_at ?? 0,
        // 直接透传节点状态，不在边界上压缩成布尔（否则 error / disabled 会丢）
        status: n.status,
        metadata: v.metadata ?? {},
        // 选项定义随节点下发（`node.schema`）；清单一次取全，面板零回读
        schema: v.schema,
      }
    })
}

/**
 * 删除会话（连同其全部消息）。
 *
 * **走 VDFS**：`delete(<根>/session/<id>)`。后端 provider 的 `delete` 与曾经的
 * `session/clear` 路由共用同一份实现（`delete_session_internal`），因此这不是
 * 换一种删除方式，而是**同一个删除**换一个入口——专用路由已退役。
 *
 * 语义差异（有意接受）：VDFS 侧先做存在性校验，删不存在的会话返回 `NotFound`；
 * 旧路由是静默成功。调用方（`stores/sessions.ts::deleteSession`）本就只在
 * 清单里找得到的会话上调用，因此这条差异在真实路径上不可达——而"删不存在的东西
 * 报错"比"静默成功"更诚实。
 */
export async function deleteSession(sessionId: string, mountDir?: string): Promise<void> {
  await deleteVdfs(vdfsSessionAddr(mountDir ?? (await ensureSessionMountDir()), sessionId))
}

/**
 * 经 VDFS 读取整份转写（会话叶子的内容是一份 JSON 文档）。
 *
 * 文档形状由后端 `session_content` 决定（`{ id, title, metadata, messages, updated_at }`）；
 * 这里只取 `messages`，其余字段由会话清单节点（`<根>/session/<id>`）承载。
 *
 * 放在本文件的理由：**文档形状的知识属于会话域**，不该出现在 store 里——
 * store 只需要 `ChatMessage[]`。读失败（空内容 / 非 JSON）在此就地转成错误，
 * 调用方不必各自写一遍 `JSON.parse` 的 try/catch。
 */
export async function readSessionTranscript(
  sessionId: string,
  mountDir?: string
): Promise<SessionMessage[]> {
  const addr = vdfsSessionAddr(mountDir ?? (await ensureSessionMountDir()), sessionId)
  const content = await readVdfs(READBACK_REASON.TRANSCRIPT_LOAD, addr)
  const text = content?.text
  if (!text) throw new Error(`读取会话转写失败：${addr}`)
  let doc: unknown
  try {
    doc = JSON.parse(text)
  } catch (e) {
    throw new Error(`会话转写不是合法 JSON（${addr}）：${e}`)
  }
  const messages = (doc as { messages?: unknown })?.messages
  return Array.isArray(messages) ? (messages as SessionMessage[]) : []
}

/**
 * 删除单条消息的**回执**。
 *
 * `deleted_ids` 是后端给出的**权威**被删列表（`vdfs/action` 的 `data`）——
 * store 用它做幂等对齐：本地若因锚点缺失等原因删窄了，据它补齐。
 * 这也是「截断」走 `action` 而不是 `delete` 的收益之一
 * （`vdfs/delete` 只回 `{path}`，带不回这个列表）。
 */
export interface DeleteMessageResult {
  deleted_ids: string[]
}

/**
 * 清空会话历史消息（保留会话本身 / 工作目录 / 标题等元数据）。
 *
 * **走 VDFS**：`action(<根>/session/<id>/message, "clear")`。
 *
 * 为什么是 `action` 而不是 `delete`：`delete` 的语义是**逐节点**的「这一个没了」，
 * 表达不了截断那类集合操作；而转写区段的删除因此统一走动作——**同一个区段的删除
 * 只有一种入口形态**，使用者不必记「哪种删除走哪个入口」。
 *
 * 实时通知**逐条下发**：每条被删的消息各发一条 `deleted` 变更（ADR-025 后消息的
 * 实时面就在 VDFS 变更上），消费端据此就地移除，不必整份重读；回执里的
 * `deleted_ids` 才是权威列表。见后端 `symbio_core::vdfs` 的
 * `VDFS_ACTION_TRUNCATE` 文档。
 */
export async function clearMessages(sessionId: string, mountDir?: string): Promise<void> {
  const scheme = await ensureSessionScheme(mountDir)
  await runVdfsAction(vdfsMessagesAddr(scheme, sessionId), VDFS_ACTION_CLEAR)
}

/**
 * 删除单条会话消息（连同其之后的所有消息一并删除）。
 *
 * **走 VDFS**：`action(<根>/session/<id>/message/<mid>, "truncate")`。语义是
 * 「从这条到列表末尾全没了」，不是「删这一个」——VDFS 的变更词汇里**没有**对应
 * 取值，这类操作一条变更都不发（理由见 `clearMessages`）。
 *
 * 目标消息不存在时后端返回空列表且**不发变更**——「什么都没删」不该在 VDFS 上
 * 留下痕迹。回执照常返回，调用方的幂等对齐因此是空操作。
 *
 * 「截断」不是变更词汇里的取值——它是**动作**；落在变更面上的是被删各条的
 * `deleted`（逐条，无 `delta`）。
 */
export async function deleteMessage(
  sessionId: string,
  messageId: string,
  mountDir?: string
): Promise<DeleteMessageResult> {
  const scheme = await ensureSessionScheme(mountDir)
  const res = await runVdfsAction(
    vdfsMessageAddr(scheme, sessionId, messageId),
    VDFS_ACTION_TRUNCATE
  )
  return {
    deleted_ids: Array.isArray(res.data)
      ? (res.data as unknown[]).filter((x): x is string => typeof x === 'string')
      : [],
  }
}

/**
 * 更新单条会话消息（手工编辑 / 标错重试等）。
 *
 * **走 VDFS**：`write(<根>/session/<id>/message/<mid>)`，请求体是消息的**字段子集**
 * （JSON 浅合并，未提供的字段保持不变）。后端 provider 只覆盖补丁里出现的字段。
 *
 * 这不是「发言」：它不触发任何编排，只是对**既有**节点的一次存储改写。新增消息
 * 仍只有聊天协议一个入口（后端对消息路径的 `create` 意图一律驳回）。
 */
export async function updateMessage(
  sessionId: string,
  message: SessionMessage,
  mountDir?: string
): Promise<void> {
  const scheme = await ensureSessionScheme(mountDir)
  await writeVdfs(vdfsMessageAddr(scheme, sessionId, message.id), JSON.stringify(message))
}

/**
 * 合并写入会话 metadata（workdir / title / agent_id 等）。
 *
 * **走 VDFS**：`write(<根>/session/<id>)`，请求体即
 * `{ metadata?, title? }`。后端 provider 的 `write` 对这两个字段实现**浅合并**
 * （未提供的字段保持不变）——`Session::merge_metadata_object` 是这件事的**唯一**
 * 实现，因此不存在"两条路径各自漂移"的窗口。
 *
 * 曾经的 `session/update` 路由**已于 2026-09-23 退役**：它唯一多出来的能力是
 * 「客户端指定会话 id」，而 VDFS 对**具名目标 + `create`** 的约定本来就是
 * 「不存在则就地创建、名字即身份」——CLI 因此改走
 * `vdfs/write(<根>/session/<id>, {create:true, metadata})`，一次调用即 upsert。
 * 会话与消息的增删改查现在**全部**在 VDFS 上。
 */
export async function updateSession(
  sessionId: string,
  metadata: SessionMetadata,
  title?: string,
  mountDir?: string
): Promise<void> {
  await writeVdfs(
    vdfsSessionAddr(mountDir ?? (await ensureSessionMountDir()), sessionId),
    JSON.stringify({ metadata, ...(title ? { title } : {}) })
  )
}
