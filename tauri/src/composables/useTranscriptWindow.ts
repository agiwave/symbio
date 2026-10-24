/**
 * 会话流的**渲染窗口** —— 首屏只铺最近若干条根节点，更早的按需展开。
 *
 * ## 它要解决的到底是什么
 *
 * 转写的读入口是整份会话文档（`vdfs/read` 一次拿全），一次 `hydrateTranscript`
 * 就把全部历史装进 store。这条链路本身是对的（权威副本在存储，增量由 VDFS 变更
 * 收敛），代价落在**渲染**：`messageTree` 有多少根节点就铺多少个 `MessageNode`，
 * 而每个节点下面还挂着工具骨架、思考、正文的整棵子树。几千条历史的会话打开时
 * 付的是「一次性建几千棵子树 + 一次性排版」，这个成本与用户当下要看的东西无关——
 * 他打开会话想看的是**最后几轮**。
 *
 * 本 composable 因此只管一件事：**这一帧交出去哪些根节点**。消息的内容、顺序、
 * 树结构一律不碰（那些在 `useChatConnection`），store 也不碰（不复制第二份转写）。
 *
 * ## 两条形状判据（不是随便选的，改之前先看它们）
 *
 * 1. **窗口起点只往左移，不往右移。**
 *    窗口锚在「本次挂载时首屏该从哪一条开始」，此后新到达的节点一律往**尾部**加。
 *    如果反过来按「最近 K 条」滑动，流式期间每来一条根节点就有一条从**头部**被卸掉
 *    ——那正是用户视线所在位置的上方，浏览器会把视口往上拽：表现为「我在翻前面的
 *    记录，界面每轮自己跳一下」。DOM 上限因此不是每帧恒定的 K，而是
 *    「首屏 K + 这次挂载里新增长的」，后者只随用户自己的活动变，正是本档想要的界。
 * 2. **窗口以根节点为单位，不以消息为单位。**
 *    一个根节点带着自己整棵子树，展开/收起都在它内部；按条切会把一棵树劈成
 *    「上半截在窗口里、下半截没了」，而按根切最多是「整轮不在」——后者是缺内容，
 *    前者是坏内容。
 *
 * 「显示更早」因此是一条**显式入口**（顶部那颗按钮），不是滚到顶就自动续：
 * 自动续会把「往上翻」变成一场无限加载，而用户需要知道自己还没看到头。
 */

import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'
import type { ChatNode } from '@/schemas/chat_node'

/** 首屏铺的根节点数（一条响应 = 一个根，约等于「最近二十来轮」） */
export const TRANSCRIPT_WINDOW_INITIAL = 40

/** 一次「显示更早」多铺的根节点数 */
export const TRANSCRIPT_WINDOW_STEP = 40

export interface TranscriptWindow {
  /** 交给模板渲染的那一段根节点（恒以尾部结束） */
  renderedRoots: ComputedRef<ChatNode[]>
  /** 还没铺出来的条数（= 窗口起点索引；0 表示已经到顶） */
  hiddenCount: ComputedRef<number>
  /** 当前铺出来的条数（按会话记忆的就是它，见 `stores/sessionUiState`） */
  renderedCount: ComputedRef<number>
  /**
   * 往前再铺一批。
   *
   * @returns 这次**实际**新铺的条数；0 = 已经到顶。调用方据此决定要不要动滚动位置
   *   （没铺出新东西却去补高度差，等于把视口凭空挪一次）。
   */
  showEarlier: () => number
}

/**
 * @param roots 全部根节点（`useChatConnection.messageTree`，只读）
 * @param initialRendered 首屏条数；按会话还原时传回上次的记录，缺省用
 *   `TRANSCRIPT_WINDOW_INITIAL`
 */
export function useTranscriptWindow(
  roots: Ref<ChatNode[]> | ComputedRef<ChatNode[]>,
  initialRendered: number = TRANSCRIPT_WINDOW_INITIAL,
): TranscriptWindow {
  /**
   * 窗口起点的**索引**；`null` = 还没锚定。
   *
   * 「还没锚定」与「起点是 0」必须是两个值：挂载时转写常常还是空的（面板与数据
   * 到位有个先后），此时按 0 锚死就等于整份历史一帧全铺——本模块要防的就是这个。
   */
  const head = ref<number | null>(null)

  watch(
    () => roots.value.length,
    (len) => {
      if (head.value !== null || len === 0) return
      head.value = Math.max(0, len - Math.max(1, initialRendered))
    },
    // **同步**：默认的观察者排到微任务，等于「数据到位后的那一帧整份历史全铺」——
    // 本模块要防的就是这一帧。锚点必须在长度变化的当场定下来。
    { immediate: true, flush: 'sync' },
  )

  const startIndex = computed(() => head.value ?? 0)

  const renderedRoots = computed(() => {
    const all = roots.value
    // 尚未锚定（还没有根节点）时交回原数组——它就是空的
    if (head.value === null) return all
    return all.slice(head.value)
  })

  const hiddenCount = computed(() => Math.min(startIndex.value, roots.value.length))

  const renderedCount = computed(() => roots.value.length - startIndex.value)

  function showEarlier(): number {
    const from = startIndex.value
    const to = Math.max(0, from - TRANSCRIPT_WINDOW_STEP)
    if (to === from) return 0
    head.value = to
    return from - to
  }

  return { renderedRoots, hiddenCount, renderedCount, showEarlier }
}
