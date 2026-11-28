// @vitest-environment happy-dom
/**
 * HotkeySheet — 快捷键速查面板单测（happy-dom）
 *
 * 本面板的全部价值是**它不是第二份清单**：内容直接读 `useHotkeys` 的登记表。
 * 于是这里钉的就是「同源」这件事本身，共三条：
 *
 * 1. 上屏的键位与说明**就是登记表里的那几条**（谁另写一份对照表，这里立刻红）；
 * 2. **每次打开重读**——登记表是动态的（列表移动、类别切换由工作台控件挂载时
 *    登记），模块加载时算好的那份会在别的页面上列出按不动的键；
 * 3. **分组序 = 首次出现序**，不按字母排：用户读的是「先全局、再本页」这个顺序，
 *    字母序会把「全局」甩到「资源列表」后面。
 *
 * 派发正确性不在本文件（那是 `composables/__tests__/useHotkeys.spec.ts`）。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import HotkeySheet from '../HotkeySheet.vue'
import { registerHotkeys, type HotkeyBinding } from '@/composables/useHotkeys'

let disposers: (() => void)[] = []

function bind(...bindings: HotkeyBinding[]) {
  disposers.push(registerHotkeys('test', bindings))
}

const b = (over: Partial<HotkeyBinding> & { id: string; keys: string }): HotkeyBinding => ({
  label: over.id,
  group: '测试',
  run: vi.fn(),
  ...over,
})

afterEach(() => {
  for (const off of disposers) off()
  disposers = []
})

/** 关着挂载：面板的清单是**打开那一刻**读出来的，所以「先关后开」才是真实时序 */
function sheet(): VueWrapper {
  return mount(HotkeySheet, { props: { visible: false } })
}

/** 上屏的行：`[键位, 说明]` */
function rows(w: VueWrapper): [string, string][] {
  return w.findAll('.hk-row').map((r) => [r.find('.hk-keys').text(), r.find('.hk-label').text()])
}

describe('速查表 ⇄ 登记表：同一份数据', () => {
  it('★ 上屏的就是登记表里的几条（键位按平台写法、说明用绑定原文）', async () => {
    bind(
      b({ id: 'palette', keys: 'mod+k', label: '打开命令面板', group: '全局' }),
      b({ id: 'down', keys: 'down', label: '下一条', group: '资源列表' })
    )
    const w = sheet()
    await w.setProps({ visible: true })

    expect(rows(w)).toEqual([
      ['Ctrl K', '打开命令面板'],
      ['↓', '下一条'],
    ])
  })

  it('分组按**首次出现序**排，不按字母序', async () => {
    bind(
      b({ id: 'a', keys: 'mod+k', label: '甲', group: '资源列表' }),
      b({ id: 'b', keys: 'mod+j', label: '乙', group: '全局' })
    )
    const w = sheet()
    await w.setProps({ visible: true })

    expect(w.findAll('.hk-group-name').map((n) => n.text())).toEqual(['资源列表', '全局'])
  })

  it('登记表后来才多出来的绑定，下一次打开就在（打开时机决定清单内容）', async () => {
    bind(b({ id: 'a', keys: 'mod+k', label: '甲', group: '全局' }))
    const w = sheet()

    await w.setProps({ visible: true })
    expect(rows(w).map((r) => r[1])).toEqual(['甲'])

    // 页面控件挂载即登记、卸载即注销——这里模拟「换了一页，多出一条本页才有的键」
    bind(b({ id: 'c', keys: 'mod+e', label: '丙', group: '编辑' }))
    await w.setProps({ visible: false })
    await w.setProps({ visible: true })

    expect(rows(w).map((r) => r[1])).toEqual(['甲', '丙'])
  })

  it('当前页面没有可用快捷键 ⇒ 明说，不给一张空表', async () => {
    const w = sheet()
    await w.setProps({ visible: true })

    expect(w.findAll('.hk-row')).toHaveLength(0)
    expect(w.text()).toContain('当前页面没有可用的快捷键')
  })
})
