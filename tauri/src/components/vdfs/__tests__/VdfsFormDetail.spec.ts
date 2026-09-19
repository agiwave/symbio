/**
 * VdfsFormDetail — VDFS `form` 渲染器的**传参基线**（happy-dom）
 *
 * 本渲染器只做两件事：把详情定义适配成 DetailForm 的 `binding: 'option'` 通道，
 * 并把**页面算好的**机制动作（重命名 / 删除）与取值原样下传。
 *
 * 它**不自行判断**机制动作该不该出现——那是页面的单点职责
 * （`useVdfs.mechanismActions`）；机制动作与定义动作的合并、同 id 去重则归
 * `mergeDetailActions`（见 `schemas/__tests__/vdfs-form.spec.ts`）。
 *
 * 故这里只锁定一条：「谁生产、谁消费，中间这一跳是直通的」——传参不被吞掉、
 * 不被改写、也不凭空补一个。去重规则若在此处再实现一遍，就又回到 G1 的三份实现。
 */
// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import VdfsFormDetail from '../VdfsFormDetail.vue'
import DetailForm from '../DetailForm.vue'
import type { DetailAction, DetailDefinition, VdfsNode } from '@/schemas/vdfs'

function formNode(schema: Partial<DetailDefinition>): VdfsNode {
  return {
    path: '.vdfs/model/p1',
    name: 'p1',
    title: 'P1',
    kind: 'model',
    status: 'active',
    access: 'rw',
    ext: 'form',
    schema: { binding: 'upload', sections: [], ...schema },
  }
}

describe('VdfsFormDetail 传参基线（通道适配 + 直通）', () => {
  it('机制动作与忙态原样下传给 DetailForm，渲染器不做二次判断', () => {
    const actions: DetailAction[] = [{ id: 'delete', label: '删除', style: 'danger' }]
    const w = mount(VdfsFormDetail, {
      props: {
        node: formNode({}),
        data: null,
        mechanismActions: actions,
        mechanismBusy: 'delete',
      },
    })

    const form = w.findComponent(DetailForm)
    expect(form.props('mechanismActions')).toEqual(actions)
    expect(form.props('mechanismBusy')).toBe('delete')
  })

  it('未注入机制动作时下传空集——渲染器不自带任何机制动作', () => {
    const w = mount(VdfsFormDetail, {
      props: { node: formNode({}), data: null },
    })

    expect(w.findComponent(DetailForm).props('mechanismActions')).toEqual([])
    expect(w.findComponent(DetailForm).props('mechanismBusy')).toBeNull()
  })

  it('取值显式下传：read 解析结果 → DetailForm 的 values，节点原样 → node', () => {
    const node = formNode({})
    const w = mount(VdfsFormDetail, {
      props: { node, data: { base_url: 'https://api' } },
    })
    const form = w.findComponent(DetailForm)
    expect(form.props('values')).toEqual({ base_url: 'https://api' })
    // 节点原样下传（同一份 VdfsNode，不再映射成另一种摘要形状）
    expect(form.props('node')).toEqual(node)
  })
})
