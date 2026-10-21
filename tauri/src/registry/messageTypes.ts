/**
 * 消息呈现域的**入口** —— 消费方只认这一个名字
 *
 * ## 分层（与 `registry/vdfsTypes.ts` 同一套）
 *
 * - `schemas/chat_node.ts`      数据契约：取值词表（常量 + 派生类型）+ 形状，零呈现知识
 * - `messageFacts.ts`           摊平与谓词：节点 → 词表取值 → `MessageFacets`，零文案
 * - `messagePresentation.ts`    呈现与文案：facets → 图标 / 标题 / 标签 / 折叠策略 / 渲染器标识
 * - `messageRouting.ts`         注册表与派生节点：标识 → 组件的登记表、typing 占位合成
 * - `messageRenderers.ts`       装配点：唯一 import 组件的地方（把标识绑到具体组件）
 * - `components/MessageNode.vue` 等  消费方：只做「把节点摊成 facets → 查表 → 渲染」
 *
 * 新增一种消息类型的代价仍是「`NODE_KINDS` 加一个取值 + 呈现表加一行」，
 * **不需要去改渲染组件里的 if 链**。
 *
 * ## 为什么入口要保留成一个小文件
 *
 * 拆的是**文件**，不是**出口**：`@/registry/messageTypes` 这个名字就是「呈现判定的集中地」，
 * 机制审计的文案（M-001「取值集中到 registry/messageTypes」、M-006「用 registry/messageTypes
 * 的判定」）与全部消费方都指着它。三个分文件之间是**单向依赖**
 * （`facts` ← `presentation` ← `routing`），下层不许 import 上层，也不越过本入口互相串门。
 *
 * 入口只做转发，不含任何判定——转发处一旦长出逻辑，分层就白拆了。
 */

export * from './messageFacts'
export * from './messagePresentation'
export * from './messageRouting'
