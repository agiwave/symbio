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
import type { DetailAction, DetailDefinition, VdfsItem } from '@/schemas/vdfs'

function formNode(schema: Partial<DetailDefinition>): VdfsItem {
  return {
    path: '@vfs/model/p1',
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
    // 节点原样下传（同一份条目，不再映射成另一种摘要形状）
    expect(form.props('node')).toEqual(node)
  })

  it('provider 漏挂 schema 时兜底空 sections——DetailForm 不再白屏', () => {
    // 机制健壮性回归：早期 sandbox 档案节点漏挂 schema，DetailForm 直接
    // 遍历 definition.sections 抛 TypeError。此处锁定渲染器这一跳的兜底。
    const node = formNode({})
    node.schema = { binding: 'option', actions: [] } as unknown as DetailDefinition
    const w = mount(VdfsFormDetail, { props: { node, data: null } })
    const def = w.findComponent(DetailForm).props('definition')
    expect(def.sections).toEqual([])
    expect(def.binding).toBe('option')
  })

  it('只读档案保留 copy 与导出，剥掉保存和删除', () => {
    const node = formNode({
      actions: [
        { id: 'save', label: '保存', style: 'primary' },
        { id: 'copy', label: '复制为可编辑档案', style: 'secondary' },
        { id: 'export', label: '导出', style: 'secondary' },
        { id: 'delete', label: '删除', style: 'danger' },
      ],
    })
    node.access = 'r'
    const w = mount(VdfsFormDetail, { props: { node, data: {} } })
    expect(w.findComponent(DetailForm).props('definition').actions?.map((a) => a.id)).toEqual([
      'copy',
      'export',
    ])
  })

  it('cap.mutable 驱动统一只读态，并说明系统管理不可编辑', async () => {
    const node = formNode({
      sections: [{
        fields: [
          { key: 'name', label: '名称', widget: 'text' },
          { key: 'enabled', label: '启用', widget: 'toggle' },
          { key: 'nested', label: '子配置', widget: 'form', form: { binding: 'option', sections: [] } },
          { key: 'items', label: '项目', widget: 'table', columns: [{ key: 'value', label: '值' }] },
        ],
      }],
      actions: [{ id: 'save', label: '保存', style: 'primary' }],
    })
    node.access = 'r'
    const w = mount(VdfsFormDetail, {
      props: { node, data: { items: [{ value: 'a' }] } },
    })
    const form = w.findComponent(DetailForm)
    expect(form.props('capabilities').mutable).toBe(false)
    expect(form.props('readonly')).toBe(true)
    expect(w.find('.readonly-note').text()).toBe('系统管理，不可编辑')
    const controls = w.findAll('.setting-item input, .setting-item select, .setting-item textarea, .setting-item button')
    expect(controls.length).toBeGreaterThan(4)
    for (const control of controls) expect(control.attributes('disabled')).toBeDefined()

    await w.setProps({ node: { ...node, access: 'rw' } })
    expect(form.props('readonly')).toBe(false)
    expect(w.find('.readonly-note').exists()).toBe(false)
    expect(w.find('input[aria-label="名称"]').attributes('disabled')).toBeUndefined()
    expect(w.find('input[aria-label="启用"]').attributes('disabled')).toBeUndefined()
    expect(w.find('button[title="配置「子配置」"]').attributes('disabled')).toBeUndefined()
    expect(w.find('.table-add').attributes('disabled')).toBeUndefined()
  })

  it('★ 保存失败只显示错误，保留当前表单与用户输入', async () => {
    const node = formNode({
      sections: [{ fields: [{ key: 'name', label: '名称', widget: 'text' }] }],
      actions: [{ id: 'save', label: '保存', style: 'primary' }],
    })
    const w = mount(VdfsFormDetail, { props: { node, data: { name: '服务端值' } } })
    const input = w.find<HTMLInputElement>('input[aria-label="名称"]')
    await input.setValue('用户输入')
    expect(w.emitted('dirty-change')).toContainEqual([true])

    await w.setProps({ error: '保存失败：写入被拒绝' })

    expect(w.find('.banner-msg').text()).toBe('保存失败：写入被拒绝')
    expect(w.find<HTMLInputElement>('input[aria-label="名称"]').element.value).toBe('用户输入')
    expect(w.findComponent(DetailForm).props('values')).toEqual({ name: '服务端值' })
    expect(w.emitted('dirty-change')).toContainEqual([true])
  })

  it('set-default 经现有 action 通道转发，testing 忙态禁用动作', async () => {
    const w = mount(VdfsFormDetail, {
      props: {
        node: formNode({
          actions: [{ id: 'set-default', label: '设为默认 Provider', style: 'icon' }],
        }),
        data: {},
      },
    })
    const button = w.find<HTMLButtonElement>('button[title="设为默认 Provider"]')
    await button.trigger('click')
    expect(w.emitted('action')).toEqual([['set-default']])

    await w.setProps({ testing: true })
    expect(w.findComponent(DetailForm).props('testing')).toBe(true)
    expect(button.element.disabled).toBe(true)
    await button.trigger('click')
    expect(w.emitted('action')).toHaveLength(1)
  })

  it('草稿态剥掉 copy（没有可复制的已存在资源）', () => {
    const node = formNode({
      actions: [
        { id: 'save', label: '保存', style: 'primary' },
        { id: 'copy', label: '复制', style: 'secondary' },
      ],
    })
    node.path = ''
    const w = mount(VdfsFormDetail, { props: { node, data: null } })
    expect(w.findComponent(DetailForm).props('definition').actions?.map((a) => a.id)).toEqual(['save'])
  })

  it('表格字段错误展示行号与中文列名，未知机器 key 原样保留', () => {
    const node = formNode({
      sections: [{
        fields: [{
          key: 'rules', label: '能力约束规则', widget: 'table',
          columns: [{ key: 'kind', label: '对象' }],
        }],
      }],
    })
    const w = mount(VdfsFormDetail, {
      props: {
        node,
        data: {},
        fieldErrors: [
          { field: 'rules.1.kind', message: '请选择对象' },
          { field: 'future_field', message: '未知错误' },
        ],
      },
    })
    expect(w.find('.banner-fields code').text()).toBe('能力约束规则 · 第 2 行 · 对象')
    expect(w.findAll('.banner-fields code')[1].text()).toBe('future_field')
  })
})
