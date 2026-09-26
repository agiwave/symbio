import { shallowRef, computed, type ComputedRef, type InjectionKey } from 'vue'
import { runVdfsAction, writeVdfs } from '@/services/vdfs'
import { ensureSessionScheme } from '@/services/vdfsScheme'
import { messageTextOf, type ChatMessage, type ResumeAction } from '@/schemas/chat_message'
import { logger } from '@/utils/logger'
import { useSessionsStore } from '@/stores/sessions'
import { isBlankContentNode, isInProgressMessage } from '@/stores/sessionTranscript'
import {
  VDFS_ACTION_ABORT,
  VDFS_ACTION_CLEAR,
  VDFS_STATUS_ACTIVE,
  VDFS_STATUS_FAILED,
  VDFS_STATUS_WORKING,
  parseVdfsSessionAddr,
  vdfsInboxAddr,
  vdfsInboxItemAddr,
  vdfsMessageAddr,
  vdfsSessionAddr,
  type VdfsSessionScheme,
} from '@/schemas/vdfs'
import { isWaitingStatus } from '@/registry/messageTypes'

export interface UseChatConnectionOptions {
  /** 会话 id（地址末段）：store 里所有以会话为键的状态都用它 */
  sessionId: string
  /**
   * 会话**完整地址** = `<挂载目录>/<id>`——三个输入动作的落点。
   *
   * 与 `sessionId` 并存：前者是地址（写哪个 inbox、中止哪个节点、恢复落在哪条
   * 消息），后者是 store 的键。**只带 id 就等于把「住在哪个空间」丢掉**，
   * 而路由是 address-less 的（`chat/send` 恒落在根实例上）——那正是
   * 「子智能体空间里发的消息跑到了父智能体」的成因。
   */
  sessionAddr: string
  onSendComplete?: () => void
}

/**
 * 「会话恢复」注入键。
 *
 * 用它取代 `'resume'` 字符串键：字符串键拼错时 `inject` 静默返回 undefined，
 * 只能在运行期以一个「点了没反应」的形式暴露；改用 symbol + `InjectionKey`
 * 后，键名本身是类型的一部分，提供 / 注入两侧的签名由编译器对齐。
 *
 * 跨层传递仍用 provide/inject 而非逐层 props：恢复入口在**消息树的任意深度**
 * （子智能体里的 user_prompt、失败 ToolCall 的重试按钮），逐层透传要把一个
 * 回调穿过多层与本组件无关的容器。要收敛的是**契约的类型**，不是传递方式。
 */
export const RESUME_KEY: InjectionKey<(payload: ResumePayload) => void> = Symbol('chat-resume')

/** 会话恢复载荷（retry_turn/retry_compaction/retry/approve/reject/supply/answer 统一接口） */
export interface ResumePayload {
  /** 目标消息 ID（恢复锚点）
   *  - retry_turn：指向 Failed Turn（msg_type=Turn）
   *  - retry_compaction：指向 Failed 压缩节点（msg_type=Compression）
   *  - 其他 action：指向 ToolCall 父节点（msg_type=ToolCall） */
  targetId: string
  /** 恢复动作——词表在 `schemas/chat_message.ts`（后端 `ResumeAction` 的镜像，
   *  由 `scripts/protocol-mirror-audit.mjs` 的 C 组守着）。此处**不**再抄一份
   *  字面量联合：抄一份就等于多一份真相，改一个词要记得改两处。 */
  action: ResumeAction
  /** supply 时的补充参数（与原 args 浅合并） */
  args?: unknown
  /** reject 时的拒绝原因 */
  reason?: string
  /** answer（ask_user）时的答案对象 */
  answer?: unknown
  /** 目标会话 id（子智能体的工具调用需发回子会话） */
  targetSessionId?: string
}

export interface UseChatConnectionReturn {
  isLoading: ComputedRef<boolean>
  isWaitingApproval: ComputedRef<boolean>
  isConnected: ComputedRef<boolean>
  messageTree: ComputedRef<ChatMessage[]>
  /** 发送一条消息。会话参数（智能体 / 模型 / 模式 / 风险等级）由后端按
   *  `session.metadata` 解析——选择动作统一经会话选项栏落库，故此处不透传。
   *
   *  ⚠️ 它是**入队**：实现是往 `<A>/inbox/<消息 id>` **写一次**（ADR-026），
   *  由那个空间自己在空闲时取出才落库。因此调用返回 ≠ 消息已进流——它出现在
   *  消息列表里，是后端消费后发权威帧那一刻（前端不做乐观回显，见 `send` 实现）。
   *  等待期间的反馈是 working 状态（空白流 + working ⇒ 补一条等待骨架）。 */
  send: (message: ChatMessage) => void
  abort: () => void
  removeMessage: (messageId: string) => void
  resume: (payload: ResumePayload) => void
}

/**
 * 多会话无状态 chat connection（v2 — 无 store 写入版）
 *
 * ## 设计（重要变化）
 *
 * - **所有状态写入由全局消费端负责**，本 composable 不订阅任何通道：消息与会话
 *   运行态都走 `stores/sessionTranscriptSync`（VDFS 变更消费端——两类地址共用
 *   一个 `seq` 计数器），落地口是 `stores/sessions.ts`（`applyTranscriptMessages` /
 *   `applySessionState`）。本 composable 只在**发起动作**时做乐观置位
 *   （随后被节点状态覆盖）。
 * - 组件订阅此 hook 只是为了：
 *   1. 拿到 `send` / `abort` 两个 one-off 命令
 *   2. 拿到派生自 store 的 `isLoading` / `isWaitingApproval` 给 UI
 *   3. 拿到 `messageTree`（直接来自 store）
 *
 * ## 多实例共存
 *
 * - 每个 `ModelChatPanel` 调一次 `useChatConnection`，都只读 store，互不干扰
 * - 切换会话时 `:key="activeId"` 触发组件 remount，旧的 useChatConnection 销毁，
 *   新的 useChatConnection 创建；store 状态保持不变，UI 立即显示
 * - 多个 session 并发时，消费端把每个会话的节点状态写入 store，
 *   缩略卡 SessionCard 通过 store.sessionStatuses 实时刷新
 *
 * ## 不再需要的旧机制
 *
 * - ❌ `messagesMap`（per-instance 缓存）—— store 是唯一权威
 * - ❌ `flushMessagesToStore`（onBeforeUnmount 写回）—— store 自动维护
 * - ❌ `onUpdateMessages` 回调—— store 直接读
 * - ❌ `kind = "session"` 事件订阅—— 状态是节点属性，不是事件（S20）
 *
 * ## `removeMessage` / `markRemoved`
 *
 * - 这两个保留在 composable 内，因为 retry 时的临时移除是 UI 局部语义
 * - 用 composable 内的 `localOverrides` 过滤，不污染 store
 * - 仅在 retry 流程（短窗口）内有效，组件销毁后失效
 */
export function useChatConnection(options: UseChatConnectionOptions): UseChatConnectionReturn {
  const store = useSessionsStore()

  const { onSendComplete } = options

  // ==================== 寻址：三个输入动作的唯一落点 ====================
  //
  // 会话的输入**没有协议路由**，全部表达为「对这个会话地址的一次写入 / 动作」：
  //
  //   send(msg)   → write(<A>/inbox/<msg.id>)            入队，空间空闲时自己消费
  //   abort()     → action(<A>, "abort")                 中止正在跑的那一轮
  //                 + action(<A>/inbox, "clear")         并清掉还没被消费的排队
  //   resume(p)   → action(<A>/message/<p.targetId>, <动作>)   落在当时那条消息上
  //
  // 三条落点的分界与 ADR-026 已确立的口径一致，不新增概念：
  // **会话节点** = 正在跑的这一轮；**inbox 条目** = 还没成为消息的消息；
  // **message 条目** = 已经是转写里的一条消息。
  //
  // 段名（`inbox` / `message`）是**运行期数据**：由 provider 决定、按 `kind`
  // 认出来（`services/vdfsScheme`），因此这里不持有任何段名字面量。

  /**
   * 某会话的地址 + 集合段方案。
   *
   * 会话地址优先用**调用方给的**（`options.sessionAddr`——从详情页一路传下来，
   * 含「住在哪个空间」）；其它会话（子会话恢复）按 store 的寻址镜像反查。
   * 段名一律现解析：它是展示名，写死一份就是第二份真相。
   */
  async function addressingOf(
    sid: string
  ): Promise<{ addr: string; scheme: VdfsSessionScheme } | null> {
    const given = sid === options.sessionId ? options.sessionAddr : ''
    const mountDir = given ? parseVdfsSessionAddr(given)?.mountDir : store.mountDirOf(sid)
    if (!mountDir) {
      logger.error('useChatConnection', `[${sid}] 无法确定会话所在的挂载目录，动作未发出`)
      return null
    }
    try {
      const scheme = await ensureSessionScheme(mountDir)
      // 地址的挂载目录**只有一个来源**：解析出来的那个。`ensureSessionScheme`
      // 的契约是「返回的 `mountDir` 就是请求的那个」，这里显式对齐一次——
      // 否则「会话地址」与「inbox 地址」会各自从一处取值，两边一旦漂移就是
      // 「消息写到了另一个空间」，而那正是本次要修的那类 bug。
      return { addr: vdfsSessionAddr(mountDir, sid), scheme: { ...scheme, mountDir } }
    } catch (err) {
      // 段名解析不出来（该挂载下还没有任何会话可供推导）⇒ 没有可写的 inbox。
      // 不猜段名：写到一个拼错的地址上比失败更难查。
      logger.error('useChatConnection', `[${sid}] 会话地址方案未就绪（${mountDir}）`, err)
      return null
    }
  }

  // 用于 `removeMessage`（本地用户主动删除一条消息的 UI 交互）
  const localOverrides = shallowRef<Record<string, Set<string>>>({}) // sessionId -> set of removed msgIds
  function getRemovedSet(): Set<string> {
    return localOverrides.value[options.sessionId] || new Set()
  }
  function markRemoved(messageId: string) {
    const next = { ...localOverrides.value }
    const cur = new Set(next[options.sessionId] || new Set())
    cur.add(messageId)
    next[options.sessionId] = cur
    localOverrides.value = next
  }

  // 从 store 派生 messageTree（按 parent_id 组织成树）
  //
  // 流式期间的性能关键：每个 token 都会触发本 computed 重算。若每次都
  // `{ ...msg }` 新建全部节点对象，keyed v-for 的 props 引用全变，整棵
  // 消息列表每帧全量重渲染。因此维护一个「上一轮节点」缓存：**子树**未变的
  // 消息直接复用旧节点对象，Vue 只重渲染真正变化的那一条路径。
  //
  // ## 签名必须覆盖**整棵子树**，而不是「自身字段 + 直接子节点 id」
  //
  // 复用节点对象的唯一目的，是让下一次 v-for 的 `:node` 保持**同一个引用**：
  // 引用不变 ⇒ Vue 判定 props 未变 ⇒ 整个组件子树跳过更新
  // （`hasPropsChanged`）。这条优化成立的**前提**是「引用不变 ⇒ 渲染结果不变」，
  // 而渲染结果取决于**整棵子树**——容器节点（Turn / ToolCall）自身无正文，
  // 它画出来的每一块内容都在子节点里。
  //
  // 旧签名只覆盖自身字段与直接子 id（`childIds.join(',')`），于是「**已有**子节点的
  // 正文增长」不改变任何祖先的签名：祖先对象原样复用 → Vue 跳过整棵子树 →
  // 增量**永远进不了 DOM**。界面只在「结构变化（新增/删除子节点）或状态迁移」
  // 那一刻跳一下，把此前积攒的内容一次补齐——实测症状正是
  // 「流式期间正文/思考不动，正文结束后突然完整；手动刷新也完整」。
  //
  // `append` 窄帧（正文 / 思考 / 工具参数 / 工具响应增长的唯一通道）恰恰只改
  // 叶子内容，不碰任何容器字段，因此这条缺陷会命中**每一帧**。
  //
  // 所以签名按子树归纳：自身字段 + 每个子节点的签名（子节点签名同样含它自己的
  // 子树）。任一后代变化 ⇒ 沿途祖先签名变化 ⇒ 这条路径换新对象 ⇒ 只重渲染这条
  // 路径；其余子树引用不变，照旧跳过（优化不丢）。会话切换时整体失效。
  let nodeCacheSession = ''
  const nodeCache = new Map<string, ChatMessage>()
  function nodeSignature(
    msg: ChatMessage,
    parentId: string | undefined,
    children: ChatMessage[] | undefined,
  ): string {
    // type / name 必须进签名：同一条消息在流式过程中可能出现 type 迁移
    // （reasoning → text）或工具名补全，漏掉它们会让缓存的旧节点对象
    // 被复用，渲染器按旧 type 分派（Reason 块不渲染 / 工具名缺失）。
    // id 也在签名里：子节点签名以其 id 打头，「换成一个同内容的子节点」同样
    // 必须使祖先签名变化（否则被换掉的子节点在界面上永远不出现）。
    //
    // `meta` / `error` 同样必须进签名——**它们都会改变渲染结果**，而帧可以
    // 「只改这两者、不动其它字段」：
    // - `meta.started_at`（工具开始执行那一刻写入）：它只随一条状态帧到达，
    //   该帧不改内容也不改状态 ⇒ 签名不变 ⇒ 缓存节点被复用 ⇒ 前端读到的
    //   `meta` 仍是旧值 ⇒ **「运行中 12s」的秒数永远不出现**（工具行只显示
    //   「运行中」，用户无从判断是还在跑还是卡住了），直到下一次真实内容变化
    //   才补上——那时工具往往已经结束；
    // - `meta.recoverable` / `failure_kind` / `prompt` / `success`：分别决定
    //   重试入口、兜底文案、待响应表单、结果成败，同样都由「只改 meta」的帧
    //   下发；
    // - `error`：失败原因文本。
    // meta 是后端 flatten 出来的浅 JSON，序列化成本远低于一次错误的全量重渲染。
    const meta = msg.meta === undefined ? '' : JSON.stringify(msg.meta)
    const own = `${msg.id}|${msg.type ?? ''}|${msg.name ?? ''}|${msg.status ?? ''}|${parentId ?? ''}|${messageTextOf(msg.content)}|${msg.error ?? ''}|${meta}`
    const sub = children
      ? children.map((child) => (child as { __sig?: string }).__sig ?? '').join(',')
      : ''
    return `${own}#${sub}`
  }
  function reuseNode(
    msg: ChatMessage,
    parentId: string | undefined,
    children: ChatMessage[] | undefined,
  ): ChatMessage {
    const sig = nodeSignature(msg, parentId, children)
    const cached = nodeCache.get(msg.id)
    if (cached && (cached as { __sig?: string }).__sig === sig) return cached
    const node: ChatMessage = { ...msg }
    ;(node as { __sig?: string }).__sig = sig
    nodeCache.set(msg.id, node)
    return node
  }

  const messageTree = computed<ChatMessage[]>(() => {
    const sessionId = options.sessionId
    if (!sessionId) return []
    if (nodeCacheSession !== sessionId) {
      nodeCacheSession = sessionId
      nodeCache.clear()
    }
    const all = store.getSessionMessages(sessionId)
    const removed = getRemovedSet()
    const filtered = all.filter(m => !removed.has(m.id))

    // 识别「空内容叶子节点」：流模式下后端会先发一个 content 为空（如 "\n\n"）的
    // Text / Reasoning 节点（例如 reasoning 与 tool_call 之间的占位空节点），
    // 该节点并不持久化，但前端会短暂收到；渲染时直接过滤掉，避免对话流出现空白块。
    // 仅过滤"无子节点"的节点——带子节点的节点是容器（Turn/ToolCall），须保留。
    // 「空内容」判据走 `sessionTranscript.isBlankContentNode`（唯一实现）：
    // 等待骨架的兜底判定也要用它，两处不能各写一份。
    const rawChildren: Record<string, ChatMessage[]> = {}
    filtered.forEach(msg => {
      if (msg.parent_id) {
        if (!rawChildren[msg.parent_id]) rawChildren[msg.parent_id] = []
        rawChildren[msg.parent_id].push(msg)
      }
    })
    const parentsWithChildren = new Set(
      Object.keys(rawChildren).filter(pid => rawChildren[pid].length > 0),
    )
    const isEmptyLeaf = (msg: ChatMessage) =>
      !parentsWithChildren.has(msg.id) && isBlankContentNode(msg)

    const visible = filtered.filter(m => !isEmptyLeaf(m))

    const childrenMap: Record<string, ChatMessage[]> = {}
    visible.forEach(msg => {
      if (msg.parent_id) {
        if (!childrenMap[msg.parent_id]) childrenMap[msg.parent_id] = []
        childrenMap[msg.parent_id].push(msg)
      }
    })

    // 判「是不是根」原来是 `visible.find(...)` —— 每个节点都要线性扫一遍，
    // 整棵树 O(n²)；长会话（数千条）在流式期间每帧都要付一次。换成一次性建
    // 好的 id 集合，查表 O(1)。
    const visibleIds = new Set(visible.map(m => m.id))
    const rootMessages = visible.filter(msg => {
      return !msg.parent_id || !visibleIds.has(msg.parent_id)
    })

    rootMessages.sort((a, b) => {
      const sa = a.seq ?? a.timestamp ?? 0
      const sb = b.seq ?? b.timestamp ?? 0
      return sa - sb
    })
    Object.values(childrenMap).forEach(children => {
      children.sort((a, b) => {
        const sa = a.seq ?? a.timestamp ?? 0
        const sb = b.seq ?? b.timestamp ?? 0
        return sa - sb
      })
    })

    // **先建子树、再定自身**：自身的签名要覆盖子节点的签名（见 `nodeSignature`），
    // 所以子节点必须先构造出来。
    const buildNode = (
      msg: ChatMessage,
      parentId?: string,
      parentNode?: ChatMessage,
    ): ChatMessage => {
      const kids = childrenMap[msg.id]
      const children = kids ? kids.map((child) => buildNode(child, msg.id)) : undefined
      const node = reuseNode(msg, parentId, children)
      // parent 反向引用不参与缓存签名（签名只含自身与子树），每次重挂：
      // 升为根（父被删）时要清掉旧引用，避免渲染树上溯到已移除的节点
      if (parentNode) node.parent = parentNode
      else delete node.parent
      if (children) {
        node.children = children
        // 子节点的 parent 指回本节点：本节点可能是**新造**的对象（子树有变化），
        // 旧的反向引用必须换掉，否则子节点上溯到的是上一轮的祖先。
        for (const child of children) child.parent = node
      } else {
        // 子节点全部消失（被删/被过滤）时清掉旧引用，避免缓存节点渲染幽灵子树
        delete node.children
      }
      return node
    }
    return rootMessages.map((msg) => buildNode(msg))
  })

  // 从 store 派生 isLoading / isWaitingApproval
  const isLoading = computed(() => {
    const sid = options.sessionId
    if (!sid) return false
    return store.isSessionWorking(sid)
  })

  const isWaitingApproval = computed(() => {
    const sid = options.sessionId
    if (!sid) return false
    const msgs = store.getSessionMessages(sid)
    return msgs.some(msg => isWaitingStatus(msg.status))
  })

  async function send(msg: ChatMessage) {
    const outgoing: ChatMessage = { ...msg }
    const sid = options.sessionId
    logger.info('useChatConnection', `[${sid}] Sending message`)

    // ⚠️ **不做乐观回显**（与从前相反，见下）。
    //
    // 发言 = 往会话**收件箱**写一条（`<A>/inbox/<消息 id>`），落库发生在
    // "被消费那一刻"：空间空闲时取出 → 追加存储 → 发一条权威帧。因此前端
    // **不抢先生成节点**——「消息出现在流里」就是"它已被处理"的唯一可观测证据，
    // 抢先画一条会让"已发出"与"已处理"在界面上无法区分（排队的消息看起来
    // 已经发完了）。等待期间的可视反馈由下面的 working 乐观置位给
    // （空白流 + working ⇒ `sessionLive.needsTypingRow` 会补一条等待骨架）。
    //
    // `outgoing.id` 就是**队列条目的 id**（地址末段）：后端 `enqueue_inbox`
    // 沿用调用方给的 id，因此消费时发出的**权威帧**与这条消息同 id
    // （前端按 id 合并，不会出现两条）。
    //
    // 立即置为 working（让 UI 立即反映 send 已经发出）；
    // 同时清空会话级错误：新一轮交互开始，上一次失败不再"最新"。
    // 这是**乐观置位**，随后会被后端会话节点的权威状态覆盖（零回读）。
    store.putStatus(sid, { status: VDFS_STATUS_WORKING, activity: '处理中…', outcome: undefined })
    store.setSessionError(sid, null)
    store.setSessionStatus(sid, VDFS_STATUS_WORKING)

    const target = await addressingOf(sid)
    if (!target) {
      const errText = 'Send failed: 会话地址方案未就绪（无法定位收件箱）'
      store.putStatus(sid, { status: VDFS_STATUS_FAILED, activity: '错误', outcome: 'failed' })
      store.setSessionStatus(sid, VDFS_STATUS_FAILED)
      store.setSessionError(sid, errText)
      onSendComplete?.()
      return
    }

    try {
      // 写收件箱条目 = 入队。**地址即身份**：条目 id 取自地址末段
      // （`outgoing.id`），正文就是那条消息本身（后端 `parse_inbox_message`
      // 认 `ChatMessage` 字段子集）。
      //
      // ctx 带上会话自身的 workdir：后端把它作为 `start_turn` 回退链的第一档
      // （`vdfs_host_ctx(ctx).get(WORKDIR)`），会话还没绑过 workdir 时它是唯一的来源。
      await writeVdfs(
        vdfsInboxItemAddr(target.scheme, sid, outgoing.id),
        JSON.stringify(outgoing),
        {
          ctx: {
            workdir: store.getSessionWorkdir(sid) ?? '',
            session_id: sid,
          },
        },
      )
    } catch (err: any) {
      const errText = `Send failed: ${err.message || String(err)}`
      // 请求本身失败（transport 级）：收敛为「以错误结束」这个状态值。
      // 后端若已开始工作，它的节点视图随后会覆盖这里——权威在服务端。
      store.putStatus(sid, { status: VDFS_STATUS_FAILED, activity: '错误', outcome: 'failed' })
      store.setSessionStatus(sid, VDFS_STATUS_FAILED)

      // 仅当没有任何 streaming/等待消息承载错误时，才落"会话级错误状态"（而非注入错误节点）：
      // 否则助手根级 Turn 会在 bus Error 事件中带上错误，避免"根级节点 + 会话级"重复报错。
      // 错误作为状态（不是消息树里的节点）展示在会话级错误条，许可重试（重新发送最后一条用户消息）。
      const hasStreaming = store.getSessionMessages(sid).some(isInProgressMessage)
      if (!hasStreaming) {
        store.setSessionError(sid, errText)
      }

      logger.error('useChatConnection', 'Failed to send', err)
      onSendComplete?.()
    }
  }

  /**
   * 停止：**两个动作**，作用在两个不同的对象上。
   *
   * 1. `action(<A>, "abort")`——中止**正在跑的那一轮**（收敛在途 Turn）；
   * 2. `action(<A>/inbox, "clear")`——清掉**还没被消费**的排队消息。
   *
   * 为什么必须成对：只 `abort` 不清队，消费者下一趟立刻取下一条继续跑，
   * 用户看到的是「点了停止它又跑起来了」。为什么不给 `abort` 加一个
   * `clear_inbox` 开关：机制动词保持单一职责，组合发生在调用方。
   *
   * 两者都**只发一次 IPC 且互不依赖**（`allSettled`）：其中一个失败不该让另一个
   * 不发——「停了但队列没清」与「队列清了但没停」都是需要被如实报告的状态。
   */
  function abort() {
    const sid = options.sessionId
    logger.info('useChatConnection', `[${sid}] aborting`)
    void (async () => {
      const target = await addressingOf(sid)
      if (!target) return
      const ctx = { workdir: store.getSessionWorkdir(sid) ?? '', session_id: sid }
      const [stopped, cleared] = await Promise.allSettled([
        runVdfsAction(target.addr, VDFS_ACTION_ABORT, undefined, ctx),
        runVdfsAction(vdfsInboxAddr(target.scheme, sid), VDFS_ACTION_CLEAR, undefined, ctx),
      ])
      if (stopped.status === 'rejected') {
        logger.error('useChatConnection', 'Failed to abort:', stopped.reason)
      } else if (!stopped.value.ok) {
        // 「本来就没在跑」不是错误，但也不该静默——回执的文案直接可读
        logger.warn('useChatConnection', `[${sid}] abort 未生效：${stopped.value.message}`)
      }
      if (cleared.status === 'rejected') {
        logger.error('useChatConnection', 'Failed to clear inbox:', cleared.reason)
      }
    })()
  }

  function removeMessage(messageId: string) { markRemoved(messageId) }

  /**
   * 会话恢复（retry_turn/retry_compaction/retry/approve/reject/supply/answer）。
   *
   * 内部走**消息节点上的动作**：`action(<A>/message/<targetId>, <动作>)`——
   * 动作词就是 `ResumeAction` 的线上词形（snake_case），**不另造一套**：
   * 它们是同一批语义，多一套就多一处漂移。
   *
   * 为什么不入队（不像发言那样写 inbox）：恢复必须落在**当时那条消息**上，
   * 而队列项的 id 是新的；入队会丢掉锚点。见 ADR-026。
   *
   * 后端语义（删除-重建模式）：
   * - retry_turn：删除 Failed Turn 及其所有子孙节点 → 重新走 LLM 请求
   * - retry_compaction：删除 Failed 压缩节点 → 重新执行一次上下文压缩
   *   （**不动历史**：压缩失败从不丢消息，重试只是再试一次 LLM 摘要）
   * - retry/approve/reject/supply/answer：删除旧子节点 → 重新执行工具或生成结果 → 创建新子节点
   *
   * 前端不在此处构造新消息——后端经 `vdfs` 频道广播（帧语义全在字段上：
   * `status = removed` 删旧节点，`content` / `delta` 写新节点与父节点状态），
   * 由 `sessionTranscriptSync` 与 `sessions.applyTranscriptMessages` 就地收敛。
   *
   * 会话参数：智能体 / 模型 provider 由后端 `resolve_session_params` 从
   * `session.metadata` 回退解析；`mode` / `risk_level` 随动作载荷携带
   * （与发言不同——发言的载荷只能是一条 `ChatMessage`，恢复的载荷是自由的）。
   */
  async function resume(payload: ResumePayload) {
    const targetSid = payload.targetSessionId || options.sessionId
    const sid = options.sessionId
    logger.info(
      'useChatConnection',
      `[${sid}] resume: action=${payload.action} targetId=${payload.targetId}${targetSid !== sid ? ` → ${targetSid}` : ''}`,
    )

    // 立即置为 working（让 UI 立即反映 resume 已经发出）；
    // 同一会话才切 working 状态，避免跨会话工具调用误改父会话状态。
    if (targetSid === sid) {
      store.putStatus(sid, { status: VDFS_STATUS_WORKING, activity: '处理中…', outcome: undefined })
      store.setSessionError(sid, null)
      store.setSessionStatus(sid, VDFS_STATUS_WORKING)
    }

    // 运行模式 / 风险等级：与从前同级别的回退链——会话记忆值 > 默认值。
    // 后端把 MODE / RISK_LEVEL 写入 chat ctx，continuation chat_loop 通过
    // ctx.fork() 继承。
    const mode = store.getSessionMode(targetSid)
    const riskLevel = store.getSessionRiskLevel(targetSid)

    const target = await addressingOf(targetSid)
    if (!target) {
      if (targetSid === sid) {
        store.putStatus(sid, { status: VDFS_STATUS_FAILED, activity: '错误', outcome: 'failed' })
        store.setSessionStatus(sid, VDFS_STATUS_FAILED)
      }
      return
    }

    try {
      const resp = await runVdfsAction(
        vdfsMessageAddr(target.scheme, targetSid, payload.targetId),
        payload.action,
        {
          args: payload.args ?? null,
          reason: payload.reason ?? null,
          answer: payload.answer ?? null,
          mode,
          risk_level: riskLevel,
        },
        {
          workdir: store.getSessionWorkdir(targetSid) ?? '',
          session_id: targetSid,
        },
      )
      // 防御性处理：后端 resume 臂有忙碌守卫，忙碌时回执 `ok:false` +
      // `data.status = "session_busy"`（**不是**抛错，走不到 catch）。此时必须复位
      // 前端 working 状态，否则 UI 会卡在"处理中…"且重试按钮不消失。
      const busy = !resp.ok && (resp.data as { status?: string } | undefined)?.status === 'session_busy'
      if (busy) {
        logger.warn('useChatConnection', `[${sid}] resume rejected: session_busy`)
        if (targetSid === sid) {
          // 「会话忙」是**当前事实**（空闲），不是失败：回落 `active`，
          // 不置 `failed`——否则会显示一个并不存在的失败终态。
          store.putStatus(sid, { status: VDFS_STATUS_ACTIVE, activity: '会话忙' })
          store.setSessionStatus(sid, VDFS_STATUS_ACTIVE)
        }
      } else if (!resp.ok) {
        // 其它「没做」的如实报告（如目标消息已不在）：不静默，也不冒充失败终态
        logger.warn('useChatConnection', `[${sid}] resume 未生效：${resp.message}`)
      }
    } catch (err: any) {
      logger.error('useChatConnection', 'Failed to resume:', err)
      if (targetSid === sid) {
        store.putStatus(sid, { status: VDFS_STATUS_FAILED, activity: '错误', outcome: 'failed' })
        store.setSessionStatus(sid, VDFS_STATUS_FAILED)
      }
    }
  }

  return {
    isLoading,
    isWaitingApproval,
    isConnected: computed(() => true), // 始终视为已连接（状态由全局消费端收敛）
    messageTree,
    send,
    abort,
    removeMessage,
    resume,
  }
}