/**
 * VdfsTextDetail —— 正文到达后必须**真的显示出来**（用户报的「文件内容始终为空」）
 *
 * 这一组用例锁的是「草稿 ↔ 基线」的同步判据，而不是渲染长相：
 *
 * 选中一个文件的那一刻正文还没读回来（`data` 是空串），组件先以空串挂载；
 * 读数到达时 `data` 由 `''` 变成全文。若判据写成「草稿 ≠ 新基线 ⇒ 认为用户正在
 * 编辑」，空草稿当然不等于新正文，同步被跳过，编辑器就永远停在占位符上——
 * 不报错、不打日志，只是空。判据必须拿**上一份基线**比：草稿仍等于上一份基线，
 * 才说明它没被编辑过、理应跟随。
 *
 * 另一半是反过来的保护：用户真的在编辑时，外部内容帧到达**不得**冲掉输入
 * （多端 / 多进程写入下很常见，静默丢数据）。
 */

// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import VdfsTextDetail from '../VdfsTextDetail.vue'
import type { VdfsItem } from '@/schemas/vdfs'

function file(path: string): VdfsItem {
  return {
    path,
    name: path.split('/').pop() ?? path,
    title: path.split('/').pop() ?? path,
    kind: 'file',
    status: 'active',
    access: 'rw',
    ext: 'text',
  }
}

const WD = '@vfs/session/s1/workdir'

describe('VdfsTextDetail：正文到达后显示 / 编辑中不被覆盖', () => {
  it('挂载时正文还没到（data 为空），到达后编辑器里就是它', async () => {
    const wrapper = mount(VdfsTextDetail, {
      props: { node: file(`${WD}/README.md`), data: '' },
    })

    const textarea = wrapper.get('textarea')
    expect(textarea.element.value, '到达前是空的').toBe('')

    await wrapper.setProps({ data: '# demo\n\n第一段' })
    await nextTick()

    expect(textarea.element.value).toBe('# demo\n\n第一段')
  })

  it('用户编辑中：外部内容帧到达不覆盖输入（静默丢数据比空文件更糟）', async () => {
    const wrapper = mount(VdfsTextDetail, { props: { node: file(`${WD}/a.md`), data: '旧正文' } })
    const textarea = wrapper.get('textarea')
    expect(textarea.element.value).toBe('旧正文')

    await textarea.setValue('用户正在写的内容')

    // 另一处写入（别的进程 / 另一台设备）送来整份新正文
    await wrapper.setProps({ data: '别处写入的正文' })
    await nextTick()

    expect(textarea.element.value, '编辑中的草稿优先').toBe('用户正在写的内容')
  })

  it('换节点：草稿重置为新节点的正文（同一份组件实例，:key 不保证重建）', async () => {
    const wrapper = mount(VdfsTextDetail, { props: { node: file(`${WD}/a.md`), data: 'A 的内容' } })
    expect(wrapper.get('textarea').element.value).toBe('A 的内容')

    // 选中另一个文件：路径先变，正文随后到达（与真实时序一致）
    await wrapper.setProps({ node: file(`${WD}/b.md`), data: '' })
    await nextTick()
    expect(wrapper.get('textarea').element.value).toBe('')

    await wrapper.setProps({ data: 'B 的内容' })
    await nextTick()
    expect(wrapper.get('textarea').element.value).toBe('B 的内容')
  })

  it('未编辑时不再把「同一份正文」重复写回（避免打断光标 / 触发多余 dirty）', async () => {
    const wrapper = mount(VdfsTextDetail, { props: { node: file(`${WD}/c.md`), data: 'abc' } })
    const textarea = wrapper.get('textarea')

    // 同值 setProps：draft 与基线仍然相等，值不变
    await wrapper.setProps({ data: 'abc' })
    await nextTick()

    expect(textarea.element.value).toBe('abc')
    const emitted = wrapper.emitted('dirty-change') ?? []
    expect(emitted[emitted.length - 1]).toEqual([false])
  })
})
