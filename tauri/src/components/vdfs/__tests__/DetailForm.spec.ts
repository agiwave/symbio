/**
 * DetailForm — 定义驱动通用详情渲染器单测（happy-dom）
 *
 * 覆盖机制升级新增的渲染语义：
 * - list / map 结构化控件的编辑态↔保存态序列化约定
 * - visible_when 字段条件显隐（隐藏字段不参与保存）
 * - info 绑定只读概览（static 取值来自节点顶层的扩展字段、options 值→标签）
 * - open-container 机制导航动作
 * - 表单取值 = 显式 `values` 入参（VDFS 的 read 内容），节点自身不携带正文
 */
// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import DetailForm from '../DetailForm.vue'
import type { DetailDefinition, VdfsNode } from '@/schemas/vdfs'

function def(partial: Partial<DetailDefinition>): DetailDefinition {
  return {
    binding: 'upload',
    sections: [],
    ...partial,
  }
}

/** 一个可编辑的资源节点（`name` 即父目录内的路径段 = 保存时的 id） */
function node(partial: Partial<VdfsNode> & { name: string }): VdfsNode {
  return {
    path: `.vdfs/${partial.name}`,
    title: partial.name,
    kind: 'mcp',
    status: 'active',
    access: 'rw',
    ...partial,
  }
}

describe('DetailForm upload 绑定：list / map 序列化', () => {
  it('预填把数组/对象转为编辑文本，保存时序列化回结构', async () => {
    const definition = def({
      sections: [
        {
          fields: [
            { key: 'args', label: '参数', widget: 'list' },
            { key: 'env', label: '环境变量', widget: 'map' },
          ],
        },
      ],
      actions: [{ id: 'save', label: '保存', style: 'primary' }],
    })

    const w = mount(DetailForm, {
      props: {
        definition,
        node: node({ name: 'srv' }),
        // 取值来自 vdfs/read 的 JSON 解析结果（显式入参，不在节点上）
        values: { args: ['a', 'b'], env: { A: '1' } },
        capabilities: { mutable: true },
      },
    })

    const textareas = w.findAll('textarea')
    expect(textareas).toHaveLength(2)
    expect((textareas[0].element as HTMLTextAreaElement).value).toBe('a\nb')
    expect((textareas[1].element as HTMLTextAreaElement).value).toBe('A=1')

    await textareas[0].setValue('x\n y \n\nz')
    await textareas[1].setValue('K=V\nE=')
    await w.find('button[title="保存"]').trigger('click')

    const payload = w.emitted('save')![0][0] as Record<string, unknown>
    // upload 绑定额外带上 id（= 节点名），其余就是序列化后的字段值
    expect(payload.id).toBe('srv')
    expect(payload.args).toEqual(['x', 'y', 'z'])
    expect(payload.env).toEqual({ K: 'V', E: '' })
  })
})

describe('DetailForm visible_when：字段条件显隐', () => {
  const transportDef = () =>
    def({
      sections: [
        {
          fields: [
            {
              key: 'type',
              label: '传输类型',
              widget: 'select',
              options: [
                { value: 'stdio', label: 'stdio' },
                { value: 'http', label: 'HTTP' },
              ],
            },
            { key: 'command', label: '命令', widget: 'text', visible_when: { key: 'type', equals: 'stdio' } },
            { key: 'url', label: 'URL', widget: 'text', visible_when: { key: 'type', not_equals: 'stdio' } },
          ],
        },
      ],
      actions: [{ id: 'save', label: '保存', style: 'primary' }],
    })

  it('条件不满足的字段整行不渲染；切换后按需出现', async () => {
    const w = mount(DetailForm, {
      props: { definition: transportDef(), node: null, capabilities: { mutable: true } },
    })
    expect(w.text()).not.toContain('命令')
    expect(w.text()).toContain('URL')

    await w.find('select').setValue('stdio')
    expect(w.text()).toContain('命令')
    expect(w.text()).not.toContain('URL')
  })

  it('隐藏字段不参与保存（stdio/http 互斥字段不互相污染）', async () => {
    const w = mount(DetailForm, {
      props: { definition: transportDef(), node: null, capabilities: { mutable: true } },
    })
    await w.find('select').setValue('stdio')
    await w.findAll('input[type="text"]')[0].setValue('npx')
    await w.find('select').setValue('http')
    await w.find('input[type="text"]').setValue('https://example.com')
    await w.find('button[title="保存"]').trigger('click')

    const payload = w.emitted('save')![0][0] as Record<string, unknown>
    expect(payload.type).toBe('http')
    expect(payload.url).toBe('https://example.com')
    expect(payload.command).toBeUndefined()
  })
})

describe('DetailForm info 绑定：只读概览 + open-container', () => {
  const infoDef = () =>
    def({
      binding: 'info',
      sections: [
        {
          fields: [
            {
              key: 'scope',
              label: '来源层级',
              widget: 'static',
              options: [
                { value: 'workspace', label: '工作区级' },
                { value: 'global', label: '全局级' },
              ],
            },
            { key: 'count_prompt', label: '提示词', widget: 'static' },
          ],
        },
      ],
      actions: [
        { id: 'open-container', label: '浏览内部', style: 'primary', payload: { kind: 'agent' } },
        { id: 'delete', label: '删除', style: 'icon danger' },
      ],
    })

  /** 概览字段是节点顶层的扩展字段（后端 flatten 下发），不是另一份摘要形状 */
  const bundleNode = node({
    name: 'b1',
    title: 'B1',
    kind: 'agent',
    path: '.vdfs/agent/b1',
    scope: 'workspace',
    count_prompt: 2,
  })

  it('static 字段从节点顶层取值，options 做值→标签映射', () => {
    const w = mount(DetailForm, {
      props: { definition: infoDef(), node: bundleNode, capabilities: { mutable: true } },
    })
    expect(w.text()).toContain('工作区级')
    expect(w.text()).toContain('2')
    // info 绑定无保存语义
    expect(w.emitted('save')).toBeUndefined()
  })

  it('open-container 动作携带 payload.kind 上抛，delete 走机制通道', async () => {
    const w = mount(DetailForm, {
      props: { definition: infoDef(), node: bundleNode, capabilities: { mutable: true } },
    })
    const buttons = w.findAll('.ea-btn')
    await buttons[0].trigger('click')
    await buttons[1].trigger('click')
    expect(w.emitted('open-container')).toEqual([['agent']])
    expect(w.emitted('delete')).toHaveLength(1)
  })

  it('mechanismActions 注入与定义动作同排（divider 分隔），机制删除走 busy 通道', async () => {
    const definition = def({
      sections: [],
      actions: [{ id: 'save', label: '保存', style: 'primary' }],
    })
    const w = mount(DetailForm, {
      props: {
        definition,
        node: node({ name: 'b1', title: 'B', kind: 'agent' }),
        capabilities: { mutable: true },
        mechanismActions: [
          { id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' },
        ],
        // 机制动作忙态由页面单点给出（替换旧的 `deleting` 布尔）
        mechanismBusy: 'delete',
      },
    })
    // 图标优先渲染：tooltip（title）= 动作名 / 进行中 busy_label
    const titles = w.findAll('.ea-btn').map((b) => b.attributes('title'))
    // 定义动作在前，机制动作经 divider 注入在后
    expect(titles).toEqual(['保存', '删除中…'])
    expect(w.find('.ea-divider').exists()).toBe(true)
  })

  it('定义未声明动作时仅渲染注入的机制动作', () => {
    const w = mount(DetailForm, {
      props: {
        definition: def({ sections: [] }),
        node: node({ name: 's1', title: 'S', kind: 'session' }),
        capabilities: { mutable: true },
        mechanismActions: [
          { id: 'open-container', label: '浏览内部（子会话）', style: 'primary', payload: { kind: 'session' } },
        ],
      },
    })
    const titles = w.findAll('.ea-btn').map((b) => b.attributes('title'))
    expect(titles).toEqual(['浏览内部（子会话）'])
    expect(w.find('.ea-divider').exists()).toBe(false)
  })
})

describe('DetailForm disabled_when：条件成立才禁用', () => {
  const deleteOnly = {
    id: 'delete',
    label: '删除',
    style: 'danger',
    disabled_when: { key: 'is_existing', equals: false },
  }

  function mountWith(target: VdfsNode | null) {
    return mount(DetailForm, {
      props: {
        definition: def({ sections: [], actions: [deleteOnly] }),
        node: target,
        capabilities: { mutable: true },
      },
    })
  }

  it('已存在节点：条件不成立 ⇒ 按钮可点', () => {
    const w = mountWith(node({ name: 'srv' }))
    expect(w.find<HTMLButtonElement>('.ea-btn').element.disabled).toBe(false)
  })

  it('新建态（无节点）：条件成立 ⇒ 按钮禁用', () => {
    const w = mountWith(null)
    expect(w.find<HTMLButtonElement>('.ea-btn').element.disabled).toBe(true)
  })

  it('无 disabled_when ⇒ 一律不禁用（缺省语义不能翻成禁用）', () => {
    const w = mount(DetailForm, {
      props: {
        definition: def({ sections: [], actions: [{ id: 'delete', label: '删除', style: 'danger' }] }),
        node: node({ name: 'srv' }),
        capabilities: { mutable: true },
      },
    })
    expect(w.find<HTMLButtonElement>('.ea-btn').element.disabled).toBe(false)
  })
})

describe('DetailForm option 绑定（VDFS）：异步 values 到达触发预填', () => {
  // VDFS 走 option 绑定，`values` 来自 `vdfs/read`、在挂载后才异步返回。
  // 若 watch 的身份键不纳入「数据是否到位」，则「挂载（null）→ 数据到达（对象）」
  // 会被门闩判成同一身份而 early-return，预填永不执行
  // ⇒ 表现为：标题回落 title_fallback、字段全空。
  const optionDef = () =>
    def({
      binding: 'option',
      title_from: ['name'],
      title_fallback: '新建 Provider',
      subtitle_from: ['provider'],
      sections: [
        {
          fields: [
            { key: 'name', label: '名称', widget: 'text' },
            {
              key: 'provider',
              label: '提供商',
              widget: 'select',
              options: [
                { value: 'openai', label: 'OpenAI' },
                { value: 'anthropic', label: 'Anthropic' },
              ],
            },
            { key: 'model', label: '模型', widget: 'datalist' },
          ],
        },
      ],
      actions: [{ id: 'save', label: '保存', style: 'primary' }],
    })

  const vdfsNode = node({ name: 'gpt4', title: 'gpt4', kind: 'model' })

  it('挂载时 values 为 null → 标题回落 fallback、字段空；read 返回后预填生效', async () => {
    const w = mount(DetailForm, {
      props: {
        definition: optionDef(),
        node: vdfsNode,
        values: null,
        capabilities: { mutable: true },
      },
    })

    // 挂载瞬间：数据未到位，预填被跳过
    expect(w.find('.title-text').text()).toBe('新建 Provider')
    expect((w.findAll('input[type="text"]')[0].element as HTMLInputElement).value).toBe('')
    expect(w.find('select').element.value).toBe('')

    // 模拟 vdfs/read 异步返回
    await w.setProps({ values: { name: 'My GPT', provider: 'openai', model: 'gpt-4o' } })

    // 数据到位后：身份键翻转（…:0 → …:1），触发表单预填
    expect(w.find('.title-text').text()).toBe('My GPT')
    const values = w
      .findAll('input[type="text"]')
      .map((i) => (i.element as HTMLInputElement).value)
    expect(values).toContain('My GPT')
    expect(values).toContain('gpt-4o')
    expect(w.find('select').element.value).toBe('openai')
  })

  it('同节点后台刷新（values 换新对象但身份键不变）→ 保留编辑现场，不重填', async () => {
    const w = mount(DetailForm, {
      props: {
        definition: optionDef(),
        node: vdfsNode,
        values: { name: 'A', provider: 'openai', model: 'a-m' },
        capabilities: { mutable: true },
      },
    })
    expect(w.find('.title-text').text()).toBe('A')

    // 同节点（仍 gpt4）values 换成新对象：键仍是 model:gpt4:1 → 不重新预填，
    // 编辑现场被保留（此处无用户编辑，故维持首次预填值 A，而非 A-REFRESH）
    await w.setProps({ values: { name: 'A-REFRESH', provider: 'anthropic', model: 'a-m2' } })
    expect(w.find('.title-text').text()).toBe('A')
  })

  it('option 绑定保存只交回纯字段值（不带 id，也不被动作载荷污染）', async () => {
    const w = mount(DetailForm, {
      props: {
        definition: def({
          binding: 'option',
          sections: [{ fields: [{ key: 'interval', label: '间隔', widget: 'number', default: 30 }] }],
          actions: [{ id: 'save', label: '保存', style: 'primary', payload: { skip_validation: true } }],
        }),
        node: vdfsNode,
        values: { interval: 7 },
        capabilities: { mutable: true },
      },
    })
    await w.find('button[title="保存"]').trigger('click')
    const payload = w.emitted('save')![0][0] as Record<string, unknown>
    expect(payload).toEqual({ interval: 7 })
  })
})
