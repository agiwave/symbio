/**
 * 消息的**分派与注册表** —— 「facets → 渲染器标识 → 组件」
 *
 * 与 VDFS 详情侧同一套机制（`registry/factory`）：本文件只声明**本域的两个约定**——
 * 兜底键（`fallback`）与标识 → 组件的登记表；「标识 → 组件 + 兜底」那套机制不在这里
 * 再实现一遍。组件本身由 `registry/messageRenderers.ts` 这一处装配点登记（唯一
 * import 组件的地方）。
 *
 * 于是「新增一种呈现形态」的完整代价是：
 *   1. wire 侧：后端枚举 + 前端 `NODE_KINDS` 各加一值（闭集跨栈批次）；若只是既有五值的
 *      呈现变体（思考 vs 压缩、整轮 vs 工具），**前端零改动**——后端在交付出口把它折成
 * `title` / `icon` / `actions` 数据即可（D-070 §决策 3）；
 *   2. `messagePresentation.ts` 的 `TYPE_PRESENTATION` 加一行（仅当新增的是第六个形态）、
 *      文案表加一行；
 *   3. `messageRenderers.ts` 加一行注册。
 * **不需要改渲染组件里的 if 链**。漏掉第 2 步会**编译报错**（`Record<NodeKind, …>`
 * 要求穷尽），而不是运行期静默退化成兜底。
 *
 * ## 还住在这一层的 typing 占位合成
 *
 * 后端**从不发** typing 帧；「生成中」的即时反馈由前端按一条规则合成一个派生节点。
 * 它和渲染器标识是同一件事的两面——都是「这个位置该出现哪个格」，所以放在一起：
 *
 * - **合成**：根级骨架 streaming 且无真实子 ⇒ 产一条 typing；
 * - **替换**：它的第一个真实子到达 ⇒ 不再是「无子」，下一帧起不产 typing；
 * - **父收束**：根级骨架不再 streaming ⇒ 谓词不成立，typing 消失。
 *
 * 之所以能靠「不再合成」实现移除，是因为 typing 是**派生节点**、从不落地——它没有
 * 需要在 store 里就地标 `status = removed` 的持久身；移除 ⟺ 谓词转假。合成节点带稳定
 * 前缀 id（`typingIdOf`），永不与后端 id 相撞。
 *
 * **只认根级骨架，不认挂在父节点下的子骨架**：后者有它自己的在途信号（「运行中 / 秒数」），
 * 给它挂「生成中」占位是错位的呈现。全程只按形状（形态 + 有无父）判定，不引入任何
 * 「一整轮」的业务概念。
 */

import type { Component } from 'vue'
import {
  NODE_KIND_SKELETON,
  NODE_KIND_TYPING,
  NODE_STATUS_STREAMING,
  type ChatNode,
} from '@/schemas/chat_node'
import { isStreamingStatus, messageTypeOf, type MessageFacets } from './messageFacts'
import { messageRendererKey, type MessageRenderer } from './messagePresentation'
import { createRendererRegistry } from './factory'


// ==================== Typing 占位合成（[]，纯函数） ====================
//
// 后端**从不发** typing 帧（见 exit）；「生成中」的即时反馈由前端按一条规则
// 合成：一个**根级骨架容器**（`type = skeleton` 且无父节点）正在 streaming、且**还没有
// 任何真实子节点**时，给它挂一个 typing 占位当第一个孩子。三条状态都由这一个谓词推出：
//
// - **合成**：根级骨架 streaming 且无真实子 ⇒ 产一条 typing；
// - **替换**：它的第一个真实子到达 ⇒ 不再是「无子」，下一帧起不产 typing
//   （真实子就是那句里的「非-typing 兄弟」，到达即顶掉占位）；
// - **父收束**：根级骨架不再 streaming ⇒ 谓词不成立，typing 消失。
//
// 之所以能靠「不再合成」实现移除，是因为 typing 是**派生节点**、从不落地——它没有
// 需要在 store 里就地标 `status=removed` 的持久身；移除 ⟺ 谓词转假。合成节点带稳定
// 前缀 id（`typingIdOf`），永不与后端 id 相撞。
//
// **只认根级骨架，不认挂在父节点下的子骨架**：后者有它自己的在途信号（「运行中 / 秒数」），
// 给它挂「生成中」占位是错位的呈现；「等 LLM 吐出第一个节点」这个语义只属于会话流的根
// ——与 `SkeletonNode` 既有的等待态判据同一 scope。全程只按形状（形态 + 有无父）判定，
// 不引入任何「一整轮」的业务概念。

/** typing 占位的稳定合成 id——以宿主为键，跨帧不变（缓存/键控都靠它）。 */
export function typingIdOf(hostId: string): string {
  return `__typing__${hostId}`
}

/** 该节点是否是 typing 的宿主——正在 streaming 的**根级骨架**（`skeleton` 且无父节点）。 */
export function isTypingHost(node: ChatNode): boolean {
  // 骨架格同时容纳会话流的根与挂在父节点下的子骨架：后者有它自己的「运行中 / 秒数」信号，
  // 不该再挂「生成中」占位。用「无父」把两者分开——纯形状判据，不读 style_tag、不认业务名。
  return (
    isStreamingStatus(node.status) &&
    messageTypeOf(node) === NODE_KIND_SKELETON &&
    !node.parent_id
  )
}

/** 从可见节点扁平表合成 typing 占位子节点（宿主 = streaming 且无真实子的根级骨架）。 */
export function synthesizeTypingChildren(visible: ChatNode[]): ChatNode[] {
  // 只数**真实子**：已有的 typing 派生节点不能反过来让宿主显得「有子」。
  const parentsWithRealChild = new Set<string>()
  for (const n of visible) {
    if (n.parent_id && messageTypeOf(n) !== NODE_KIND_TYPING) {
      parentsWithRealChild.add(n.parent_id)
    }
  }
  const out: ChatNode[] = []
  for (const host of visible) {
    if (!isTypingHost(host)) continue
    if (parentsWithRealChild.has(host.id)) continue
    out.push({
      id: typingIdOf(host.id),
      parent_id: host.id,
      type: NODE_KIND_TYPING,
      status: NODE_STATUS_STREAMING,
      principal: host.principal,
      seq: host.seq,
    })
  }
  return out
}

/**
 * 消息域的渲染器注册表（机制来自 `registry/factory`）。
 *
 * 本文件只声明**本域的两个约定**：兜底键（`fallback`）与「标识 → 组件」这张表的
 * 所有权；标识的联合类型 `MessageRenderer` 与「该用哪个标识」的判定住在
 * `messagePresentation.ts`（呈现表的一行就是它的取值来源），「标识 → 组件 + 兜底」
 * 那套机制不在这里再实现一遍。
 */
const renderers = createRendererRegistry<MessageRenderer>('fallback')

/** 为某个渲染器登记组件（UI 资产，与数据契约严格分离） */
export const registerMessageRenderer = renderers.register

/** 取已登记的渲染器组件（未登记返回 undefined，调用方负责兜底） */
export const getMessageRenderer = renderers.get

/**
 * 解析节点该由哪个组件渲染（**前端选择渲染形态的唯一入口**）。
 *
 * 未登记的渲染器一律回落到 `fallback`——这样「后端先行上线了新 `type`、
 * 前端还是旧版」不会把会话流渲染成空白。`fallback` 本身也没登记时返回
 * `undefined`，由视图层决定如何兜底（正常构建下不会发生）。
 */
export function resolveMessageRenderer(f: MessageFacets): Component | undefined {
  return renderers.resolve(messageRendererKey(f))
}

