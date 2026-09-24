/**
 * VdfsMessageDetail — VDFS `message` 渲染器单测（happy-dom）
 *
 * 一条消息是**转写列表的一项**，它的分工是：**正文在内容里（`data`），
 * 结构在 `attributes` 里**（role / type / parent_id / seq / error / meta）。
 * 本单测锁定这条分工在前端的落点：
 *
 * 1. 正文取 `data`（追加型变更就地拼进同一个缓冲，因此视图无需重读即可跟着长）；
 * 2. 角色 / 类型 / 状态 / 错误来自 `node` 的扩展字段；
 * 3. **没有写入口**——发言是一次动作（走聊天协议），不是一次文件写入。
 */
// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import VdfsMessageDetail from '../VdfsMessageDetail.vue'
import type { VdfsItem } from '@/schemas/vdfs'

function messageNode(partial: Partial<VdfsItem> = {}): VdfsItem {
  return {
    path: '@vfs/session/abc/message/m1',
    name: 'm1',
    title: '助手',
    kind: 'file',
    status: 'streaming',
    access: 'r',
    ext: 'message',
    role: 'assistant',
    type: 'text',
    seq: 7,
    ...partial,
  }
}

describe('VdfsMessageDetail（转写列表项的只读视图）', () => {
  it('正文来自节点内容；结构来自 attributes', () => {
    const w = mount(VdfsMessageDetail, {
      props: { node: messageNode(), data: '你好，世界' },
    })
    expect(w.find('.msg-body').text()).toBe('你好，世界')
    expect(w.find('.role').text()).toBe('助手')
    expect(w.find('.seq').text()).toBe('#7')
    expect(w.find('.status').text()).toBe('生成中')
  })

  it('类型缺省（text）不显示标签，其余类型显示中文名', () => {
    const plain = mount(VdfsMessageDetail, { props: { node: messageNode(), data: 'x' } })
    expect(plain.find('.type').exists()).toBe(false)

    const tool = mount(VdfsMessageDetail, {
      props: { node: messageNode({ type: 'tool_call', role: 'assistant' }), data: '' },
    })
    expect(tool.find('.type').text()).toBe('工具调用')
  })

  it('失败消息把错误原因显示出来（错误是状态，也是这条消息的终态）', () => {
    const w = mount(VdfsMessageDetail, {
      props: {
        node: messageNode({ status: 'failed', error: '上游 429' }),
        data: '半截回复',
      },
    })
    expect(w.find('.msg-error').text()).toBe('上游 429')
    expect(w.find('.status').text()).toBe('失败')
  })

  it('组合节点无正文时给出说明而不是空白', () => {
    const w = mount(VdfsMessageDetail, {
      props: { node: messageNode({ type: 'turn' }), data: '' },
    })
    expect(w.find('.msg-body').exists()).toBe(false)
    expect(w.find('.msg-empty').exists()).toBe(true)
  })

  it('只读：不提供保存 / 重命名 / 删除入口（写入入口唯一）', () => {
    const w = mount(VdfsMessageDetail, {
      props: { node: messageNode(), data: '正文' },
    })
    expect(w.findAll('button')).toHaveLength(0)
  })
})
