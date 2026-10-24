// @vitest-environment happy-dom
/**
 * useHotkeys — 快捷键**机制**单测
 *
 * 本文件钉的是机制本身：判键规则、三条守卫、派发序、速查表与登记表同源。
 * 「某条键按下去做了什么」属于宿主，由宿主的用例钉（外壳见
 * `views/__tests__/MainLayout.spec.ts`，列表与停止见各自组件的用例）——
 * 机制层不认识任何具体绑定，所以这里也不出现它们。
 *
 * 判据按「写错时谁静默失效」排：
 *
 * 1. **判键**：`mod` 的平台差、大小写、`digit` 这类「一条绑定多个命中」、
 *    修饰组合必须恰好相等（多按一个 Shift 就不是同一个键）、
 *    以及键盘上必须按 Shift 才出得来键面的那几键（`mod+/`）不得因 Shift 失配。
 * 2. **三条守卫**：打字优先（只管裸键）、模态让路、`when` 不成立 ⇒ **不吞键**。
 *    三者错了都不报错，只是那键按不出来 / 按错了地方。
 * 3. **注册序即优先级**，且注销真的生效——登记表是模块级单例，漏注销会让
 *    上一个组件实例继续抢键（这条在真实页面表现为「切换页面后快捷键失灵」）。
 * 4. **速查表与绑定同源**：`hotkeyTable()` 就是登记表本身，不是第二份清单。
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import {
  dispatchHotkeys,
  formatHotkey,
  hotkeyTable,
  matchesHotkey,
  parseHotkey,
  registerHotkeys,
  useHotkeys,
  type HotkeyBinding,
} from '../useHotkeys'

let disposers: (() => void)[] = []

/** 登记一批绑定，用例结束自动注销（登记表是模块级单例，不清会串用例） */
function bind(owner: string, bindings: HotkeyBinding[]) {
  disposers.push(registerHotkeys(owner, bindings))
}

function el(html: string): HTMLElement {
  const host = document.createElement('div')
  host.innerHTML = html
  document.body.appendChild(host)
  return host.firstElementChild as HTMLElement
}

/**
 * 打一次键：有目标就从那个元素冒泡（守卫读的就是 target 是谁）。
 *
 * 只负责**造事件**，派发仍由用例显式调 `dispatchHotkeys`——真实页面里那是
 * `MainLayout` 挂在 window 上的监听器（见其单测）。在这里手动补一刀，
 * 「同一次按键只应答一条绑定」这类断言才会被重复派发污染。
 */
function press(over: Partial<KeyboardEventInit> & { key: string }, target?: HTMLElement) {
  const evt = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...over })
  ;(target ?? document.body).dispatchEvent(evt)
  return evt
}

const binding = (over: Partial<HotkeyBinding> & { id: string; keys: string }): HotkeyBinding => ({
  label: over.id,
  group: '测试',
  run: vi.fn(),
  ...over,
})

afterEach(() => {
  for (const off of disposers) off()
  disposers = []
  document.body.replaceChildren()
  vi.unstubAllGlobals()
})

describe('parseHotkey — 键规格', () => {
  it('mod + 单键', () => {
    expect(parseHotkey('mod+k')).toEqual({ mod: true, alt: false, key: 'k' })
  })

  it('修饰可以叠加，顺序不参与语义（只有最末一段是键）', () => {
    expect(parseHotkey('mod+alt+delete')).toEqual({ mod: true, alt: true, key: 'delete' })
  })

  it('裸键不带修饰；空白与大小写不敏感', () => {
    expect(parseHotkey(' Esc ')).toEqual({ mod: false, alt: false, key: 'escape' })
  })

  it('别名收成协议词，方向键归一', () => {
    expect(parseHotkey('up').key).toBe('arrowup')
    expect(parseHotkey('space').key).toBe(' ')
    expect(parseHotkey('mod+digit').key).toBe('digit')
  })
})

describe('matchesHotkey — 这次按键算不算命中', () => {
  const modK = parseHotkey('mod+k')

  it('mod 在 mac 上是 ⌘、在其它平台上是 Ctrl（判定不看平台名）', () => {
    expect(matchesHotkey(modK, press({ key: 'k', ctrlKey: true }))).toBe(true)
    expect(matchesHotkey(modK, press({ key: 'k', metaKey: true }))).toBe(true)
  })

  it('大写锁定（`e.key` 是 "K"）是同一个键', () => {
    expect(matchesHotkey(modK, press({ key: 'K', ctrlKey: true }))).toBe(true)
  })

  it('修饰组合必须恰好相等：多按一个 Shift / Alt 就不是这条键', () => {
    expect(matchesHotkey(modK, press({ key: 'k', ctrlKey: true, shiftKey: true }))).toBe(false)
    expect(matchesHotkey(modK, press({ key: 'k', ctrlKey: true, altKey: true }))).toBe(false)
    expect(matchesHotkey(modK, press({ key: 'k' }))).toBe(false)
  })

  it('digit 是**一类键**：1–9 命中，0 不命中', () => {
    const spec = parseHotkey('mod+digit')
    for (const d of ['1', '5', '9']) expect(matchesHotkey(spec, press({ key: d, ctrlKey: true }))).toBe(true)
    expect(matchesHotkey(spec, press({ key: '0', ctrlKey: true }))).toBe(false)
  })

  /**
   * ★ 键面本身要按 Shift 才出得来（美式键盘的 `/`、`?`）⇒ Shift 不算多余修饰。
   *
   * 少了这条，`mod+/` 在美式键盘上**永远命不中**——因为打不出不带 Shift 的 `/`。
   */
  it('隐含 Shift 的键面不因 Shift 失配，也不要求必须按 Shift', () => {
    const spec = parseHotkey('mod+/')
    expect(matchesHotkey(spec, press({ key: '/', ctrlKey: true, shiftKey: true }))).toBe(true)
    expect(matchesHotkey(spec, press({ key: '/', ctrlKey: true }))).toBe(true)
  })
})

describe('dispatchHotkeys — 派发与三条守卫', () => {
  it('命中即消费：preventDefault 后把事件交给绑定', () => {
    const run = vi.fn()
    bind('shell', [binding({ id: 'palette', keys: 'mod+k', run })])

    const evt = press({ key: 'k', ctrlKey: true })
    expect(dispatchHotkeys(evt)).toBe(true)
    expect(run).toHaveBeenCalledWith(evt)
    expect(evt.defaultPrevented, '不拦就会同时叫出系统的搜索框').toBe(true)
  })

  it('没人认领的键原样还给浏览器（既不 preventDefault 也不算消费）', () => {
    bind('shell', [binding({ id: 'palette', keys: 'mod+k', run: vi.fn() })])

    const evt = press({ key: 'j', ctrlKey: true })
    expect(evt.defaultPrevented).toBe(false)
    expect(dispatchHotkeys(evt)).toBe(false)
  })

  it('★ 注册序即优先级；注销后下一条同键绑定才接得到', () => {
    const first = vi.fn()
    const second = vi.fn()
    const off1 = registerHotkeys('a', [binding({ id: 'first', keys: 'mod+k', run: first })])
    bind('b', [binding({ id: 'second', keys: 'mod+k', run: second })])

    dispatchHotkeys(press({ key: 'k', ctrlKey: true }))
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).not.toHaveBeenCalled()

    // 注销一条**自己登记**的绑定（其余仍由 afterEach 收）
    disposers = disposers.filter((off) => off !== off1)
    off1()

    dispatchHotkeys(press({ key: 'k', metaKey: true }))
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(1)
  })

  /** 组件局部的键（输入框里的 Enter、编辑器里的 Ctrl+S）天然优先，不需要认键。 */
  it('事件已被局部处理（defaultPrevented）⇒ 全局绑定不参与', () => {
    const run = vi.fn()
    bind('shell', [binding({ id: 'palette', keys: 'mod+k', run })])

    const evt = new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true, cancelable: true })
    const input = el('<input>')
    input.addEventListener('keydown', (e) => e.preventDefault())
    input.dispatchEvent(evt)

    expect(evt.defaultPrevented).toBe(true)
    expect(dispatchHotkeys(evt)).toBe(false)
    expect(run).not.toHaveBeenCalled()
  })

  describe('守卫一：打字优先（只管裸键）', () => {
    it('裸键在输入控件里让路——那是在打字，不是在下命令', () => {
      const run = vi.fn()
      bind('list', [binding({ id: 'down', keys: 'down', run })])

      for (const html of ['<input>', '<textarea>', '<select>', '<div contenteditable="true"></div>']) {
        expect(dispatchHotkeys(press({ key: 'ArrowDown' }, el(html))), html).toBe(false)
      }
      expect(run).not.toHaveBeenCalled()
    })

    /**
     * ★ 带修饰的组合**不进**这一条：`Cmd+K` 是打不出来的字。
     *
     * 曾经的形态是所有绑定一律「焦点在输入框里就让路」，后果是命令面板在它
     * 的头号使用场景（正写着消息）里按不出来。
     */
    it('带 mod 的组合在输入框里照常命中', () => {
      const run = vi.fn()
      bind('shell', [binding({ id: 'palette', keys: 'mod+k', run })])

      expect(dispatchHotkeys(press({ key: 'k', ctrlKey: true }, el('<textarea>')))).toBe(true)
      expect(run).toHaveBeenCalledTimes(1)
    })

    it('裸键要进输入框必须显式标 allowInInput', () => {
      const run = vi.fn()
      bind('chat', [binding({ id: 'abort', keys: 'esc', run, allowInInput: true })])

      expect(dispatchHotkeys(press({ key: 'Escape' }, el('<input>')))).toBe(true)
      expect(run).toHaveBeenCalledTimes(1)
    })
  })

  describe('守卫二：模态让路', () => {
    it('落在打开着的模态里 ⇒ 全局绑定一律不参与（含带修饰的）', () => {
      const run = vi.fn()
      bind('shell', [binding({ id: 'palette', keys: 'mod+k', run })])

      const inside = el('<div class="modal-mask"><div class="modal-panel"><input></div></div>')
      expect(dispatchHotkeys(press({ key: 'k', ctrlKey: true }, inside.querySelector('input') as HTMLElement))).toBe(false)
      expect(run).not.toHaveBeenCalled()
    })

    it('模态外面仍可命中（让路只看事件落在谁身上）', () => {
      const run = vi.fn()
      bind('shell', [binding({ id: 'palette', keys: 'mod+k', run })])

      el('<div class="modal-mask"></div>')
      expect(dispatchHotkeys(press({ key: 'k', ctrlKey: true }, el('<button>')))).toBe(true)
      expect(run).toHaveBeenCalledTimes(1)
    })
  })

  describe('守卫三：when 不成立 ⇒ 不吞键', () => {
    it('前提不成立时既不执行也不 preventDefault（死键比没有这条键更糟）', () => {
      const run = vi.fn()
      bind('shell', [binding({ id: 'new', keys: 'mod+n', run, when: () => false })])

      const evt = press({ key: 'n', ctrlKey: true })
      expect(dispatchHotkeys(evt)).toBe(false)
      expect(run).not.toHaveBeenCalled()
      expect(evt.defaultPrevented).toBe(false)
    })

    it('前一条前提不成立 ⇒ 让给后面的同键绑定；`when` 拿到的是这次的事件', () => {
      const second = vi.fn()
      bind('a', [binding({ id: 'first', keys: 'mod+digit', run: vi.fn(), when: () => false })])
      bind('b', [binding({ id: 'second', keys: 'mod+digit', run: second, when: (e) => e.key === '3' })])

      expect(dispatchHotkeys(press({ key: '7', ctrlKey: true }))).toBe(false)
      expect(second).not.toHaveBeenCalled()

      expect(dispatchHotkeys(press({ key: '3', ctrlKey: true }))).toBe(true)
      expect(second).toHaveBeenCalledTimes(1)
    })
  })
})

describe('登记表 ⇄ 速查表：同一份数据', () => {
  it('hotkeyTable 就是登记表本身，注销即从中消失', () => {
    expect(hotkeyTable()).toEqual([])

    const off = registerHotkeys('shell', [
      binding({ id: 'a', keys: 'mod+k', label: '打开面板', group: '全局' }),
      binding({ id: 'b', keys: 'down', label: '下一项', group: '列表' }),
    ])
    expect(hotkeyTable().map((b) => [b.id, b.keys, b.label, b.group])).toEqual([
      ['a', 'mod+k', '打开面板', '全局'],
      ['b', 'down', '下一项', '列表'],
    ])

    off()
    expect(hotkeyTable()).toEqual([])
  })

  it('useHotkeys 在作用域销毁时自动注销（漏了就是「切页后快捷键失灵」）', () => {
    const scope = effectScope()
    scope.run(() => {
      useHotkeys('page', [binding({ id: 'x', keys: 'mod+e' })])
    })
    expect(hotkeyTable().map((b) => b.id)).toEqual(['x'])

    scope.stop()
    expect(hotkeyTable()).toEqual([])
  })

  /**
   * ★ 速查表上没有第二条键位来源。
   *
   * 面板（`HotkeySheet.vue`）渲染的就是这张表，所以键位与说明**不可能**与绑定漂移；
   * 这条断言把「同源」钉在机制侧——日后谁加一份手写清单，这里先红。
   */
  it('速查表读到的每一项都带得上屏的键位与分组', () => {
    bind('shell', [binding({ id: 'palette', keys: 'mod+k', label: '打开命令面板', group: '全局' })])

    for (const b of hotkeyTable()) {
      expect(b.keys).toBeTruthy()
      expect(b.label).toBeTruthy()
      expect(b.group).toBeTruthy()
    }
  })
})

describe('formatHotkey — 用户屏幕上按得出来的写法', () => {
  it('非 mac：修饰与键之间有空格（⌘ 那种连写是 mac 的排印习惯）', () => {
    vi.stubGlobal('navigator', { userAgent: 'Mozilla/5.0 (Windows NT 10.0)' })
    expect(formatHotkey('mod+k')).toBe('Ctrl K')
    expect(formatHotkey('mod+alt+delete')).toBe('Ctrl Alt Delete')
    expect(formatHotkey('esc')).toBe('Esc')
    expect(formatHotkey('down')).toBe('↓')
    expect(formatHotkey('mod+digit')).toBe('Ctrl 1–9')
  })

  it('mac：⌘ / ⌥ 连写', () => {
    vi.stubGlobal('navigator', { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X)' })
    expect(formatHotkey('mod+k')).toBe('⌘K')
    expect(formatHotkey('mod+alt+delete')).toBe('⌘⌥Delete')
    expect(formatHotkey('esc')).toBe('Esc')
  })
})
