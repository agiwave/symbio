/**
 * VdfsFormDetail — VDFS `form` 渲染器的机制动作注入单测（happy-dom）
 *
 * 覆盖：机制动作（删除）与详情定义声明动作的**去重**——
 * model / mcp / skill / agent 的定义都自带 `delete`（「删除 Provider」等），
 * 若机制再注入一个「删除」，同页会出现两个删除按钮（重复入口）。
 * 两者语义相同（都经 @delete → vdfs/delete），定义已声明时不再注入；
 * 定义未声明时机制仍兜底提供（写权限判据不变）。
 */
// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import VdfsFormDetail from '../VdfsFormDetail.vue'
import DetailForm from '@/components/entities/DetailForm.vue'
import type { DetailDefinition } from '@/schemas/entities'
import type { VdfsNode } from '@/schemas/vdfs'

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

function injectedActions(w: ReturnType<typeof mount>) {
  return w.findComponent(DetailForm).props('mechanismActions') as Array<{
    id: string
  }>
}

describe('VdfsFormDetail 机制动作注入去重', () => {
  it('定义已声明 delete → 不再注入机制版「删除」（避免重复按钮）', () => {
    const w = mount(VdfsFormDetail, {
      props: {
        node: formNode({
          actions: [
            { id: 'save', label: '保存', style: 'primary' },
            { id: 'delete', label: '删除 Provider', style: 'icon danger' },
          ],
        }),
        data: null,
      },
    })
    expect(injectedActions(w).some((a) => a.id === 'delete')).toBe(false)
  })

  it('定义未声明 delete → 机制仍兜底注入「删除」', () => {
    const w = mount(VdfsFormDetail, {
      props: {
        node: formNode({
          actions: [{ id: 'save', label: '保存', style: 'primary' }],
        }),
        data: null,
      },
    })
    expect(injectedActions(w).some((a) => a.id === 'delete')).toBe(true)
  })
})
