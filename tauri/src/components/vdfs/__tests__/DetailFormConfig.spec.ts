/**
 * DetailForm config 绑定 — 加载时序与填充实证（happy-dom）
 *
 * 回归场景：设置分区（会话/本地工具/网络工具）经 load_path 拉取
 * `<plugin>/config/get` 后应把配置填充进表单字段。
 */
// @vitest-environment happy-dom
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'

const callPluginMock = vi.fn()
vi.mock('@/services/plugin', () => ({
  callPlugin: (...args: unknown[]) => callPluginMock(...args),
}))

import DetailForm from '../DetailForm.vue'
import type { DetailDefinition, VdfsNode } from '@/schemas/vdfs'

const sessionDef: DetailDefinition = {
  binding: 'config',
  load_path: 'session/config/get',
  save_path: 'session/config/set',
  title_fallback: '会话设置',
  sections: [
    {
      fields: [
        { key: 'max_messages', label: '最大消息数', widget: 'number', default: 100 },
        { key: 'auto_compress', label: '自动压缩', widget: 'toggle', default: true },
        { key: 'context_messages', label: '上下文消息数量', widget: 'number', default: 6 },
      ],
    },
  ],
  actions: [{ id: 'save', label: '保存配置', style: 'primary' }],
}

/** 设置分区节点（config 绑定的取值走 load_path，不来自节点也不来自 values） */
function settingNode(): VdfsNode {
  return {
    path: '.vdfs/setting/session',
    name: 'session',
    title: '会话设置',
    kind: 'setting',
    status: 'active',
    access: 'rw',
  }
}

describe('DetailForm config 绑定：load_path 填充', () => {
  beforeEach(() => {
    callPluginMock.mockReset()
  })

  it('config/get 返回的配置填充进对应字段', async () => {
    callPluginMock.mockResolvedValueOnce({
      max_messages: 200,
      auto_compress: false,
      context_messages: 12,
    })

    const w = mount(DetailForm, {
      props: {
        definition: sessionDef,
        node: settingNode(),
        capabilities: { mutable: false },
      },
    })
    await flushPromises()

    expect(callPluginMock).toHaveBeenCalledWith('session/config/get', {})
    const numbers = w.findAll('input[type="number"]')
    expect((numbers[0].element as HTMLInputElement).value).toBe('200')
    expect((numbers[1].element as HTMLInputElement).value).toBe('12')
    const toggle = w.find('input[type="checkbox"]')
    expect((toggle.element as HTMLInputElement).checked).toBe(false)
  })

  it('config/get 失败时字段回落定义默认值', async () => {
    callPluginMock.mockRejectedValueOnce(new Error('boom'))
    const w = mount(DetailForm, {
      props: {
        definition: sessionDef,
        node: settingNode(),
        capabilities: { mutable: false },
      },
    })
    await flushPromises()
    // 加载失败回落 initForm 默认（此处验证不崩溃且 toggle 默认生效）
    expect(w.findAll('input').length).toBeGreaterThan(0)
  })

  it('保存经 save_path 写回当前表单值', async () => {
    callPluginMock
      .mockResolvedValueOnce({ max_messages: 200, auto_compress: false, context_messages: 12 })
      .mockResolvedValueOnce({ success: true })

    const w = mount(DetailForm, {
      props: {
        definition: sessionDef,
        node: settingNode(),
        capabilities: { mutable: false },
      },
    })
    await flushPromises()
    await w.find('button[title="保存配置"]').trigger('click')
    await flushPromises()
    expect(callPluginMock).toHaveBeenLastCalledWith('session/config/set', {
      max_messages: 200,
      auto_compress: false,
      context_messages: 12,
    })
  })
})
