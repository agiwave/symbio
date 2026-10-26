/**
 * 分支组读数 —— `registry/messageFacts` 的三个分支读法单测。
 *
 * 这一层是前端**唯一**认识后端 `meta.branch` / `meta.turn_root` 的地方（机制审计 M-001：
 * `components/**` 不得读 meta）。它出错的表现不是报错，是**画错控件**：
 * - 形状不齐却读出一个对象 ⇒ 渲染器画出一个点了不知道会切到哪里的切换器；
 * - `candidates` 被读成下标 ⇒ 组件开始自己算边界，切换动作发错 id；
 * - `turn_root` 被忽略 ⇒ `flat` 档升根的那批节点在藏支时漏在外面（未选中支的子孙
 *   还看得见，等于模型和界面各读一套）。
 * 所以这里按「什么形状读成 `null`」逐条钉，而不是只钉正常路径。
 */

import { describe, expect, it } from 'vitest'
import { messageBranchNeighbor, messageBranchOf, messageTurnRoot } from '../messageFacts'

/** 一条正常盖章的成员根（默认：两支里的第二支，且它正在被读）。 */
function stamped(meta: Record<string, any> = {}) {
  return {
    meta: {
      branch: {
        group: 't1',
        candidates: ['t1', 't2'],
        index: 1,
        total: 2,
        selected: true,
        ...meta,
      },
    },
  }
}

describe('messageBranchOf：只有形状齐了才读成一组', () => {
  it('读全五格（组件拿它画 `‹ 2/2 ›` 与前后是谁）', () => {
    expect(messageBranchOf(stamped())).toEqual({
      group: 't1',
      candidates: ['t1', 't2'],
      index: 1,
      total: 2,
      selected: true,
    })
  })

  it('没有标注 ⇒ null：它不在任何组里，也就没有切换器这回事', () => {
    expect(messageBranchOf({})).toBeNull()
    expect(messageBranchOf({ meta: undefined })).toBeNull()
    expect(messageBranchOf({ meta: {} })).toBeNull()
    expect(messageBranchOf({ meta: { turn_root: 't1' } })).toBeNull()
    // 后端把这一格写成非对象（脏数据 / 别的键占了这个名）⇒ 读不出任何字段
    expect(messageBranchOf({ meta: { branch: 't1' } })).toBeNull()
    expect(messageBranchOf({ meta: { branch: null } })).toBeNull()
  })

  it('形状不齐一律 null，不猜：宁可少画一个控件，也不画一个发错目标的控件', () => {
    const bad: Record<string, any>[] = [
      // candidates 与 total 不一致（后端两者同源，不齐说明这格是半截的）
      { candidates: ['t1'], total: 2 },
      // index 越界：画不出「第几支」
      { index: 2 },
      { index: -1 },
      // 类型不对：这些值进 `‹ ›` 的边界判断会静默切错
      { selected: 'true' },
      { index: '1' },
      { total: null },
      { candidates: 't1,t2' },
      { candidates: ['t1', 2] },
      { group: 7 },
      // 缺字段
      { group: undefined },
    ]
    for (const patch of bad) {
      expect(messageBranchOf(stamped(patch))).toBeNull()
    }
  })
})

describe('messageBranchNeighbor：给出的是候选 id，不是下标', () => {
  it('第二支往前一格是第一支', () => {
    const b = messageBranchOf(stamped())!
    expect(messageBranchNeighbor(b, -1)).toBe('t1')
  })

  it('走到头返回 null（首支没有更早的、末支没有更晚的）', () => {
    const first = messageBranchOf(stamped({ index: 0, selected: false }))!
    expect(messageBranchNeighbor(first, -1)).toBeNull()
    expect(messageBranchNeighbor(first, 1)).toBe('t2')
    const last = messageBranchOf(stamped())!
    expect(messageBranchNeighbor(last, 1)).toBeNull()
  })

  it('三支时逐格都指得对人（组件不再重写数组边界）', () => {
    const middle = messageBranchOf(
      stamped({ candidates: ['t1', 't2', 't3'], index: 1, total: 3 }),
    )!
    expect(messageBranchNeighbor(middle, -1)).toBe('t1')
    expect(messageBranchNeighbor(middle, 1)).toBe('t3')
  })
})

describe('messageTurnRoot：`flat` 档升根的节点靠它认回自己的轮', () => {
  it('读出记号', () => {
    expect(messageTurnRoot({ meta: { turn_root: 'tb' } })).toBe('tb')
  })

  it('缺失 / 空串都读成 null（空串是「没记」，不是「记了个空根」）', () => {
    expect(messageTurnRoot({})).toBeNull()
    expect(messageTurnRoot({ meta: {} })).toBeNull()
    expect(messageTurnRoot({ meta: { turn_root: '' } })).toBeNull()
    expect(messageTurnRoot({ meta: { turn_root: 7 } })).toBeNull()
  })
})
