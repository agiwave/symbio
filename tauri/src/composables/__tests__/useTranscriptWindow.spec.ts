/**
 * useTranscriptWindow — 会话流渲染窗口单测
 *
 * 这个模块存在的唯一理由是「打开一个几千条历史的会话时，别一次性铺几千棵子树」。
 * 它不碰数据（转写仍在 store），只决定一帧交出哪些根节点，所以判据全是**形状**判据：
 *
 * 1. **锚定发生在数据到位那一刻**。面板挂载时转写常常还是空的；如果按「此刻 0 条 ⇒
 *    起点 0」锚死，历史一到就整份铺开，本模块防的正是这件事。
 * 2. **起点只往左移，不往右移**。按「最近 K 条」滑动会让流式期间每条新根节点挤掉
 *    一条最旧的，而被挤掉的正在用户视线的**上方** ⇒ 界面每轮自己抖一下。
 * 3. **窗口恒以尾部结束**，且 `hiddenCount + renderedCount === total`（按钮上的数字
 *    与真实缺失的条数不能各算一份）。
 * 4. 到顶之后再点**不再改动**（返回 0），调用方据此不动滚动位置。
 */

import { describe, expect, it } from 'vitest'
import { ref, shallowRef } from 'vue'
import type { ChatNode } from '@/schemas/chat_node'
import {
  useTranscriptWindow,
  TRANSCRIPT_WINDOW_INITIAL,
  TRANSCRIPT_WINDOW_STEP,
} from '../useTranscriptWindow'

/** 只关心 id 与条数，节点形状造最小够用的一批 */
function roots(n: number): ChatNode[] {
  return Array.from({ length: n }, (_, i) => ({ id: `r${i}`, seq: i }) as ChatNode)
}

describe('useTranscriptWindow — 首屏锚定', () => {
  it('挂载时转写还是空的 ⇒ 起点在数据到位那一刻才定，只铺最近 K 条', () => {
    const all = shallowRef<ChatNode[]>([])
    const win = useTranscriptWindow(all)
    expect(win.renderedRoots.value, '空转写不该有一个已锚定的窗口').toEqual([])
    expect(win.hiddenCount.value).toBe(0)

    all.value = roots(100)
    expect(win.renderedRoots.value).toHaveLength(TRANSCRIPT_WINDOW_INITIAL)
    expect(win.renderedRoots.value[0].id).toBe('r60')
    expect(win.renderedRoots.value[9].id).toBe('r69')
    expect(win.hiddenCount.value).toBe(60)
  })

  it('历史不足 K 条 ⇒ 全铺，且不给「显示更早」入口', () => {
    const win = useTranscriptWindow(ref(roots(12)))
    expect(win.renderedRoots.value).toHaveLength(12)
    expect(win.hiddenCount.value).toBe(0)
    expect(win.showEarlier(), '已经没有更早的可铺').toBe(0)
  })

  it('★ K 是条下限而不是上限：一条都没铺出来时也要铺 1 条（窗口不能空转）', () => {
    // initialRendered 来自按会话的记忆，被写坏成 0 也要有一个可读的首屏
    const win = useTranscriptWindow(ref(roots(10)), 0)
    expect(win.renderedRoots.value).toHaveLength(1)
  })
})

describe('useTranscriptWindow — 起点只往左移', () => {
  it('★ 新根节点到达时，已经铺出来的那条不掉出窗口（挤掉用户上方的内容 = 界面自己抖）', () => {
    const all = ref<ChatNode[]>(roots(50))
    const win = useTranscriptWindow(all, 20)
    const firstId = win.renderedRoots.value[0].id
    expect(firstId).toBe('r30')

    // 流式期间又来了一轮：尾部增长，头部那条**必须还在**
    all.value = roots(51)
    expect(win.renderedRoots.value[0].id, '起点被往后挪了').toBe(firstId)
    expect(win.renderedRoots.value).toHaveLength(21)
    expect(win.renderedRoots.value[20].id).toBe('r50')
  })

  it('展开一批后继续增长，起点仍不动', () => {
    const all = ref<ChatNode[]>(roots(200))
    const win = useTranscriptWindow(all, 40)
    win.showEarlier()
    const startId = win.renderedRoots.value[0].id
    all.value = roots(205)
    expect(win.renderedRoots.value[0].id).toBe(startId)
  })
})

describe('useTranscriptWindow — 显示更早', () => {
  it('一次多铺 STEP 条，返回实际新增条数', () => {
    const win = useTranscriptWindow(ref(roots(100)), 40)
    expect(win.showEarlier()).toBe(TRANSCRIPT_WINDOW_STEP)
    expect(win.renderedRoots.value).toHaveLength(40 + TRANSCRIPT_WINDOW_STEP)
    expect(win.renderedRoots.value[0].id).toBe('r20')
  })

  it('最后一次不足一批 ⇒ 只铺到顶，并如实返回实际条数', () => {
    const win = useTranscriptWindow(ref(roots(55)), 40)
    // 顶部只剩 15 条
    expect(win.showEarlier()).toBe(15)
    expect(win.hiddenCount.value).toBe(0)
    expect(win.renderedRoots.value).toHaveLength(55)
  })

  it('★ 到顶之后再点是空操作（返回 0 ⇒ 调用方不动滚动位置，界面不凭空抖一下）', () => {
    const win = useTranscriptWindow(ref(roots(50)), 40)
    win.showEarlier() // 铺到顶
    expect(win.hiddenCount.value).toBe(0)
    const before = win.renderedCount.value
    expect(win.showEarlier()).toBe(0)
    expect(win.renderedCount.value).toBe(before)
  })

  it('hiddenCount 与 renderedCount 恒分成全部条数（按钮上的数字就是真实缺失的条数）', () => {
    const all = ref<ChatNode[]>(roots(137))
    const win = useTranscriptWindow(all, 40)
    expect(win.hiddenCount.value + win.renderedCount.value).toBe(137)
    win.showEarlier()
    expect(win.hiddenCount.value + win.renderedCount.value).toBe(137)
    win.showEarlier()
    expect(win.hiddenCount.value, '97 → 57 → 17').toBe(17)
    win.showEarlier()
    expect(win.hiddenCount.value + win.renderedCount.value, '到顶后仍成立').toBe(137)
    expect(win.hiddenCount.value).toBe(0)
  })
})

describe('useTranscriptWindow — 按会话还原窗口', () => {
  it('传回上次铺出来的条数 ⇒ 首屏就按那个量铺（配得上记下的离底距离）', () => {
    const win = useTranscriptWindow(ref(roots(300)), 120)
    expect(win.renderedRoots.value).toHaveLength(120)
    expect(win.renderedRoots.value[0].id).toBe('r180')
  })

  it('记住的窗口比整份历史还大 ⇒ 收敛到全铺，不越界', () => {
    const win = useTranscriptWindow(ref(roots(8)), 120)
    expect(win.renderedRoots.value).toHaveLength(8)
    expect(win.hiddenCount.value).toBe(0)
  })
})
