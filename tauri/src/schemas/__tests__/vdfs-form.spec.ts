/**
 * VDFS `form` 方言的**动作区装配**纯逻辑单测（node 环境）
 *
 * 覆盖 `mergeDetailActions`：把「渲染器自有动作」（save / test / open-container…）
 * 与「机制级默认动作」（重命名 / 删除，页面单点算好）装配成一行动作区。
 *
 * 这两段此前在 DetailForm / VdfsFormDetail / 各渲染器里**各算了一遍**（G1），
 * 同一份去重规则散在三处，改一处漏两处的概率不为零。现在是唯一实现，故直接
 * 对它下断言——规则只有两条，且都是「错了不报错、只多画或少画一个按钮」的类型：
 *
 * 1. 两段之间插一个 divider（视觉分组；缺了会把两段读成同一组）；
 * 2. **同 id 时渲染器声明的那一份胜出** —— 否则「详情定义自带 delete」与
 *    「机制兜底的 delete」会渲染成两个删除按钮（重复入口，点哪个都说不清）。
 *
 * 另有三条排序/对齐约定：自有动作在前、机制动作用单个 id 表达忙态、返回的三个
 * 数组等长（`VdfsActions` 的入参形状）。
 */

import { describe, expect, it } from 'vitest'
import { mergeDetailActions } from '../vdfs-form'
import type { DetailAction } from '../vdfs'

const save: DetailAction = { id: 'save', label: '保存', style: 'primary' }
const test: DetailAction = { id: 'test', label: '测试', style: 'secondary' }
const rename: DetailAction = { id: 'rename', label: '重命名', style: 'secondary' }
const del = (label: string): DetailAction => ({ id: 'delete', label, style: 'danger' })

describe('mergeDetailActions 装配与去重', () => {
  it('自有动作在前、机制动作经 divider 注入在后，三数组等长且按索引对齐', () => {
    const r = mergeDetailActions([save], [rename, del('删除')])

    expect(r.actions.map((a) => a.id)).toEqual(['save', 'divider', 'rename', 'delete'])
    expect(r.actions[1].style).toBe('divider')
    // `VdfsActions` 的入参形状：busy / disabled 与 actions 等长
    expect(r.busy).toHaveLength(r.actions.length)
    expect(r.disabled).toHaveLength(r.actions.length)
  })

  it('同 id 时定义声明的那一份胜出：不出现两个 delete 按钮，也不插多余的 divider', () => {
    // 详情定义已自带「删除 Provider」——机制兜底的 delete 不该再注入一份
    const r = mergeDetailActions([save, del('删除 Provider')], [del('删除')])

    const deletes = r.actions.filter((a) => a.id === 'delete')
    expect(deletes, '重复入口是这次装配要消灭的东西').toHaveLength(1)
    expect(deletes[0].label, '更具体的那份文案来自定义').toBe('删除 Provider')
    // 机制那一段被去重掉 ⇒ 没有可注入的动作 ⇒ 不插 divider
    expect(r.actions.some((a) => a.id === 'divider')).toBe(false)
  })

  it('机制动作忙态按 id 判定（同时最多一个在跑），自有动作不受影响', () => {
    const r = mergeDetailActions([save], [rename, del('删除')], [], [], 'delete')

    const busyById = Object.fromEntries(r.actions.map((a, i) => [a.id, r.busy[i]]))
    expect(busyById.delete).toBe(true)
    expect(busyById.rename).toBe(false)
    expect(busyById.save).toBe(false)
  })

  it('自有动作的忙 / 禁用按索引透传；机制动作一律不禁用（忙态已挡住重复点击）', () => {
    const r = mergeDetailActions([save, test], [del('删除')], [true, false], [false, true])

    expect(r.busy.slice(0, 2)).toEqual([true, false])
    expect(r.disabled.slice(0, 2)).toEqual([false, true])
    expect(r.disabled[r.actions.length - 1], '机制动作不参与 disabled_when').toBe(false)
  })

  it('无自有动作时不插 divider（仅机制动作）——消息 / 只读详情即此形态', () => {
    const r = mergeDetailActions([], [rename, del('删除')])

    expect(r.actions.map((a) => a.id)).toEqual(['rename', 'delete'])
    expect(r.actions.some((a) => a.id === 'divider')).toBe(false)
  })

  it('机制动作为空时只有自有动作——新建态 / 只读节点的形态', () => {
    const r = mergeDetailActions([save], [])

    expect(r.actions.map((a) => a.id)).toEqual(['save'])
    expect(r.actions.some((a) => a.id === 'divider')).toBe(false)
  })
})
