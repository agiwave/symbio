// @vitest-environment happy-dom
/**
 * `SessionChangesList` —— 变更清单的**排版器**判据。
 *
 * 这个组件刻意没有业务知识（不认识任何工具名、不翻译 `op`、不拼地址），因此它
 * 值得钉的只有两件事：
 *
 * 1. **行的每一格都来自后端**：路径 / 动作文案 / 轮号 / 次数原样呈现，组件不换算；
 * 2. **三种空分开说**：没跑过（`has_wal: false`）、无权看（`readable: false`）、
 *    没改过（两者都真但 `rows` 为空）——混成一句会把权限问题显示成「没有变更」。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import SessionChangesList from '../session/SessionChangesList.vue'
import type { ChangeRow, SessionChanges } from '@/services/sessionChanges'

function ledger(over: Partial<SessionChanges> = {}): SessionChanges {
  return {
    session_id: 's1',
    has_wal: true,
    readable: true,
    rows: [],
    ...over,
  }
}

function row(over: Partial<ChangeRow> = {}): ChangeRow {
  return {
    path: 'docs/a.md',
    address: 'workdir/docs/a.md',
    op: 'write',
    label: '写入',
    turn: 3,
    changes: 1,
    ...over,
  }
}

describe('SessionChangesList：行是呈现就绪的', () => {
  it('路径 / 文案 / 轮号 / 次数都原样取自后端；`op` 只作为判别标记', () => {
    const first = row({ changes: 2 })
    const second = row({
      path: 'src/lib.rs',
      address: 'workdir/src/lib.rs',
      op: 'edit',
      label: '编辑',
      turn: 4,
    })
    const w = mount(SessionChangesList, { props: { changes: ledger({ rows: [first, second] }) } })

    const rows = w.findAll('.change-row')
    expect(rows).toHaveLength(2)

    expect(rows[0].attributes('data-op')).toBe('write')
    expect(rows[0].text()).toContain('docs/a.md')
    expect(rows[0].text()).toContain('写入')
    expect(rows[0].text()).toContain('第 3 轮')
    expect(rows[0].text()).toContain('2 次')

    expect(rows[1].attributes('data-op')).toBe('edit')
    expect(rows[1].text()).toContain('src/lib.rs')
    expect(rows[1].text()).toContain('编辑')
    expect(rows[1].text(), '只改过一次不必写「1 次」').not.toContain('次')
  })

  it('点一行把整行原样上抛（目标地址由调用方拼，本组件不认识 `workdir`）', async () => {
    const only = row()
    const w = mount(SessionChangesList, { props: { changes: ledger({ rows: [only] }) } })

    await w.find('.change-row').trigger('click')

    expect(w.emitted('open')?.[0]?.[0]).toEqual(only)
  })
})

describe('SessionChangesList：三种空分开说', () => {
  it('没有事实源 ⇒ 「还没跑过一轮」，不是「没改过」', () => {
    const w = mount(SessionChangesList, {
      props: { changes: ledger({ has_wal: false }) },
    })
    expect(w.find('.changes-empty').text()).toContain('还没有跑过一轮')
  })

  it('有源但读方无权 ⇒ 说出来（空表会被读成「没改过」）', () => {
    const w = mount(SessionChangesList, {
      props: { changes: ledger({ readable: false }) },
    })
    expect(w.find('.changes-empty').text()).toContain('无权查看')
  })

  it('有源、可读、空 ⇒ 「还没改动过工作目录」', () => {
    const w = mount(SessionChangesList, { props: { changes: ledger() } })
    expect(w.find('.changes-empty').text()).toContain('还没有改动过工作目录')
  })
})
