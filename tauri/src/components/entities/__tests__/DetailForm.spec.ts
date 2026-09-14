/**
 * DetailForm — 定义驱动通用详情渲染器单测（happy-dom）
 *
 * 覆盖机制升级新增的渲染语义：
 * - list / map 结构化控件的编辑态↔保存态序列化约定
 * - visible_when 字段条件显隐（隐藏字段不参与保存）
 * - info 绑定只读概览（static 取值来自 item 顶层 flatten、options 值→标签）
 * - open-container 机制导航动作
 */
// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import DetailForm from '../DetailForm.vue'
import type { DetailDefinition, EntitySummary } from '@/schemas/entities'

function def(partial: Partial<DetailDefinition>): DetailDefinition {
  return {
    binding: 'upload',
    sections: [],
    ...partial,
  }
}

function existingItem(): EntitySummary {
  return {
    kind: 'mcp',
    id: 'srv',
    name: 'srv',
    status: 'active',
    config: { args: ['a', 'b'], env: { A: '1' } },
  } as EntitySummary
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
      props: { definition, item: existingItem(), capabilities: { mutable: true } as never },
    })

    const textareas = w.findAll('textarea')
    expect(textareas).toHaveLength(2)
    expect((textareas[0].element as HTMLTextAreaElement).value).toBe('a\nb')
    expect((textareas[1].element as HTMLTextAreaElement).value).toBe('A=1')

    await textareas[0].setValue('x\n y \n\nz')
    await textareas[1].setValue('K=V\nE=')
    await w.find('button[title="保存"]').trigger('click')

    const payload = w.emitted('save')![0][0] as { manifest: Record<string, unknown> }
    expect(payload.manifest.args).toEqual(['x', 'y', 'z'])
    expect(payload.manifest.env).toEqual({ K: 'V', E: '' })
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
      props: { definition: transportDef(), item: null, capabilities: { mutable: true } as never },
    })
    expect(w.text()).not.toContain('命令')
    expect(w.text()).toContain('URL')

    await w.find('select').setValue('stdio')
    expect(w.text()).toContain('命令')
    expect(w.text()).not.toContain('URL')
  })

  it('隐藏字段不参与保存（stdio/http 互斥字段不互相污染）', async () => {
    const w = mount(DetailForm, {
      props: { definition: transportDef(), item: null, capabilities: { mutable: true } as never },
    })
    await w.find('select').setValue('stdio')
    await w.findAll('input[type="text"]')[0].setValue('npx')
    await w.find('select').setValue('http')
    await w.find('input[type="text"]').setValue('https://example.com')
    await w.find('button[title="保存"]').trigger('click')

    const payload = w.emitted('save')![0][0] as { manifest: Record<string, unknown> }
    expect(payload.manifest.type).toBe('http')
    expect(payload.manifest.url).toBe('https://example.com')
    expect(payload.manifest.command).toBeUndefined()
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
        { id: 'open-container', label: '管理实体', style: 'primary', payload: { kind: 'agent' } },
        { id: 'delete', label: '删除', style: 'icon danger' },
      ],
    })

  const bundleItem = {
    kind: 'agent',
    id: 'b1',
    name: 'B1',
    status: 'active',
    scope: 'workspace',
    count_prompt: 2,
  } as unknown as EntitySummary

  it('static 字段从 item 顶层取值，options 做值→标签映射', () => {
    const w = mount(DetailForm, {
      props: { definition: infoDef(), item: bundleItem, capabilities: { mutable: true } as never },
    })
    expect(w.text()).toContain('工作区级')
    expect(w.text()).toContain('2')
    // info 绑定无保存语义
    expect(w.emitted('save')).toBeUndefined()
  })

  it('open-container 动作携带 payload.kind 上抛，delete 走机制通道', async () => {
    const w = mount(DetailForm, {
      props: { definition: infoDef(), item: bundleItem, capabilities: { mutable: true } as never },
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
        item: { kind: 'agent', id: 'b1', name: 'B', status: 'active' } as EntitySummary,
        capabilities: { mutable: true } as never,
        mechanismActions: [
          { id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' },
        ],
        deleting: true,
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
        item: { kind: 'session', id: 's1', name: 'S', status: 'active' } as EntitySummary,
        capabilities: { mutable: true } as never,
        mechanismActions: [
          { id: 'open-container', label: '管理内部实体（子会话）', style: 'primary', payload: { kind: 'session' } },
        ],
      },
    })
    const titles = w.findAll('.ea-btn').map((b) => b.attributes('title'))
    expect(titles).toEqual(['管理内部实体（子会话）'])
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

  function mountWith(item: EntitySummary) {
    return mount(DetailForm, {
      props: {
        definition: def({ sections: [], actions: [deleteOnly] }),
        item,
        capabilities: { mutable: true } as never,
      },
    })
  }

  it('已存在条目：条件不成立 ⇒ 按钮可点', () => {
    const w = mountWith({ kind: 'mcp', id: 'srv', name: 'srv', status: 'active' } as EntitySummary)
    expect(w.find<HTMLButtonElement>('.ea-btn').element.disabled).toBe(false)
  })

  it('新建态（id 为空）：条件成立 ⇒ 按钮禁用', () => {
    const w = mountWith({ kind: 'mcp', id: '', name: '', status: 'active' } as EntitySummary)
    expect(w.find<HTMLButtonElement>('.ea-btn').element.disabled).toBe(true)
  })

  it('无 disabled_when ⇒ 一律不禁用（缺省语义不能翻成禁用）', () => {
    const w = mount(DetailForm, {
      props: {
        definition: def({ sections: [], actions: [{ id: 'delete', label: '删除', style: 'danger' }] }),
        item: { kind: 'mcp', id: 'srv', name: 'srv', status: 'active' } as EntitySummary,
        capabilities: { mutable: true } as never,
      },
    })
    expect(w.find<HTMLButtonElement>('.ea-btn').element.disabled).toBe(false)
  })
})

describe('DetailForm option 绑定（VDFS）：异步 optionData 到达触发预填', () => {
  // 复现 S 系列迁移后的详情空白 bug：VDFS 走 option 绑定，optionData 来自
  // `vdfs/read`、在挂载后才异步返回。修复前 watch 身份键恒为 'option'，
  // 「挂载（null）→ 数据到达（对象）」被门闩判成同一身份而 early-return，
  // 预填永不执行 ⇒ 标题回落 title_fallback、字段全空。
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

  const vdfsItem = { kind: 'model', id: 'gpt4', name: 'gpt4' } as EntitySummary

  it('挂载时 optionData 为 null → 标题回落 fallback、字段空；read 返回后预填生效', async () => {
    const w = mount(DetailForm, {
      props: {
        definition: optionDef(),
        item: vdfsItem,
        optionData: null,
        capabilities: { mutable: true } as never,
      },
    })

    // 挂载瞬间：数据未到位，预填被跳过
    expect(w.find('.title-text').text()).toBe('新建 Provider')
    expect((w.findAll('input[type="text"]')[0].element as HTMLInputElement).value).toBe('')
    expect(w.find('select').element.value).toBe('')

    // 模拟 vdfs/read 异步返回（extra.config）
    await w.setProps({ optionData: { name: 'My GPT', provider: 'openai', model: 'gpt-4o' } })

    // 数据到位后：身份键翻转（…:0 → …:1），触发表单预填
    expect(w.find('.title-text').text()).toBe('My GPT')
    const values = w
      .findAll('input[type="text"]')
      .map((i) => (i.element as HTMLInputElement).value)
    expect(values).toContain('My GPT')
    expect(values).toContain('gpt-4o')
    expect(w.find('select').element.value).toBe('openai')
  })

  it('同条目后台刷新（optionData 换新对象但身份键不变）→ 保留编辑现场，不重填', async () => {
    const w = mount(DetailForm, {
      props: {
        definition: optionDef(),
        item: vdfsItem,
        optionData: { name: 'A', provider: 'openai', model: 'a-m' },
        capabilities: { mutable: true } as never,
      },
    })
    expect(w.find('.title-text').text()).toBe('A')

    // 同条目（仍 gpt4）optionData 换成新对象：键仍是 model:gpt4:1 → 不重新预填，
    // 编辑现场被保留（此处无用户编辑，故维持首次预填值 A，而非 A-REFRESH）
    await w.setProps({ optionData: { name: 'A-REFRESH', provider: 'anthropic', model: 'a-m2' } })
    expect(w.find('.title-text').text()).toBe('A')
  })
})
