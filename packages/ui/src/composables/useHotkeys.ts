/**
 * 快捷键的**唯一机制**：键规格 → 命中判定 → 守卫 → 派发。
 *
 * ## 为什么机制与绑定分家
 *
 * 绑定必须声明在状态所在的地方（面板显隐在 `MainLayout`、列表移动在
 * `VdfsWorkbench`），而**判键的规则只有一份**：`Cmd`/`Ctrl` 的平台差、大写锁定、
 * 非美式布局、打字时的让路——每一条写两遍就必然漂一遍。于是本文件只回答
 * 「这个键算不算命中这条绑定、该不该由它应答」，不回答「按了之后做什么」。
 *
 * ## 三条守卫（各自只在这里实现一次）
 *
 * 1. **打字优先（只管裸键）**：焦点在 `input` / `textarea` / `select` /
 *    contenteditable 里时，**不带修饰键**的绑定不参与。带 `mod` / `alt` 的组合是
 *    打不出来的字，让它们也让路会把命令面板挡在它头号使用场景之外（正写着消息
 *    时按 Cmd+K）。裸键要进输入框必须显式标 `allowInInput`——那个例外有真实代价，
 *    所以必须是显式的：`Esc` 停止生成正是**打字途中**要按的那一键。
 * 2. **模态让路**：事件落在打开着的模态（`.modal-mask`，类名契约见
 *    `components/common/BaseModal.vue`）里 ⇒ 全局绑定一律不参与。焦点陷阱与
 *    `Esc` 关闭归 BaseModal；两层同时应答会出现「关了确认框又顺手中止了会话」。
 * 3. **`when` 不满足 ⇒ 不吞键**：绑定的前提不成立（没有可切第 3 个类别、没有
 *    在跑的会话）时把键原样让给宿主。一条恒空的死键比没有这条键更糟——
 *    它还会顶掉浏览器自己的同一个组合。
 *
 * 另有一条顺带的次序判据：`e.defaultPrevented` 已为真 ⇒ 不参与。组件局部的键
 * （输入框里的 Enter、编辑器里的 Ctrl+S）因此天然优先，不需要再写一份
 * 「这是不是我的键」的识别。
 */

import { onScopeDispose } from 'vue'

/** 一条快捷键绑定 */
export interface HotkeyBinding {
  /** 稳定标识（同一注册方内唯一；重复 id 是同一张表里的笔误） */
  id: string
  /**
   * 键规格：`[mod][+alt]+键`，小写。`mod` = mac 上的 ⌘ / 其它平台的 Ctrl，
   * 判定在本文件（见 `matchesHotkey`）——绑定声明里不出现平台名。
   *
   * 键名别名：`esc` `enter` `space` `up` `down` `left` `right`；
   * `digit` 是**一类键**（1–9），用于「一条绑定、九个命中、速查表一行」。
   */
  keys: string
  /** 速查表上的一句话（用户读的就是它） */
  label: string
  /** 速查表分组（组序 = 首次出现序，不按字母排） */
  group: string
  /** 裸键在焦点落在输入控件里时仍然生效（只给「打字途中也要按」的那几键） */
  allowInInput?: boolean
  /**
   * 现在能不能按；返回 false 时**不吞键**，让给宿主。
   *
   * 拿到事件是为了「这条键位今天存不存在」这类判据：`mod+digit` 是一条绑定、
   * 九个命中，第 7 个类别不存在时数字 7 必须原样留给浏览器（Ctrl+7 是切标签页），
   * 而不是被一条什么都不做的死键吃掉。
   */
  when?: (e: KeyboardEvent) => boolean
  run: (e: KeyboardEvent) => void
}

/** 解析后的键规格（`shift` 不在这里：见 `matchesHotkey` 的那段理由） */
export interface HotkeySpec {
  mod: boolean
  alt: boolean
  key: string
}

const KEY_ALIASES: Record<string, string> = {
  esc: 'escape',
  escape: 'escape',
  enter: 'enter',
  space: ' ',
  up: 'arrowup',
  down: 'arrowdown',
  left: 'arrowleft',
  right: 'arrowright',
  digit: 'digit',
}

/** `mod+/` 里的 `/` 在美式键盘上必须按 Shift 才出得来——这种键面 shift 不算多余修饰 */
function isShiftImpliedGlyph(key: string): boolean {
  return key.length === 1 && !/^[a-z0-9]$/.test(key)
}

/** 键规格 → 判定用的形状（`1+2+3` 只有最末一段是键，前面全是修饰） */
export function parseHotkey(spec: string): HotkeySpec {
  const parts = spec.trim().toLowerCase().split('+')
  const rawKey = parts[parts.length - 1] ?? ''
  const mods = parts.slice(0, -1)
  return {
    mod: mods.includes('mod'),
    alt: mods.includes('alt'),
    key: KEY_ALIASES[rawKey] ?? rawKey,
  }
}

/** 这次按键是否命中这个规格（修饰组合必须**恰好相等**：多按 Shift/Alt 即不命中） */
export function matchesHotkey(s: HotkeySpec, e: KeyboardEvent): boolean {
  if (s.mod !== (e.ctrlKey || e.metaKey) || s.alt !== e.altKey) return false
  const key = (e.key || '').toLowerCase()
  const hit = s.key === 'digit' ? /^[1-9]$/.test(key) : key === s.key
  if (!hit) return false
  return isShiftImpliedGlyph(s.key) ? true : !e.shiftKey
}

/** 注册表元素：绑定 + 预解析的规格 + 声明它的注册方（只用于诊断） */
interface Registered {
  owner: string
  binding: HotkeyBinding
  spec: HotkeySpec
}

const registry: Registered[] = []

/**
 * 登记一批绑定，返回注销函数。
 *
 * **注册序即优先级**：外壳先注册（全局动作），页面控件后注册；两者命中同一个键时
 * 全局的先走。这不是一条隐式约定，而是派发循环的序——写在这里就是为了让人看得见。
 */
export function registerHotkeys(owner: string, bindings: HotkeyBinding[]): () => void {
  const added = bindings.map((binding) => ({ owner, binding, spec: parseHotkey(binding.keys) }))
  registry.push(...added)
  return () => {
    for (const item of added) {
      const at = registry.indexOf(item)
      if (at >= 0) registry.splice(at, 1)
    }
  }
}

/** 在组件（或 effectScope）里登记绑定，作用域销毁时自动注销 */
export function useHotkeys(owner: string, bindings: HotkeyBinding[]): void {
  onScopeDispose(registerHotkeys(owner, bindings))
}

/** 当前登记表（速查表与测试读它——**绑定只声明一次，速查表不会与它漂移**） */
export function hotkeyTable(): readonly HotkeyBinding[] {
  return registry.map((r) => r.binding)
}

function isTypingTarget(el: Element | null): boolean {
  if (!el) return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || (el as HTMLElement).isContentEditable
}

/**
 * 派发一次按键：命中第一条**可以应答**的绑定，`preventDefault` 后交出去。
 *
 * 注册序即优先级，所以第一条命中就是最外层（外壳）声明的那条。
 *
 * 返回是否被消费（供外壳与测试判定「这条键该不该归浏览器」）。
 */
export function dispatchHotkeys(e: KeyboardEvent): boolean {
  if (e.defaultPrevented) return false
  const el = e.target instanceof Element ? e.target : null
  if (el?.closest('.modal-mask')) return false
  for (const { binding, spec } of registry) {
    if (!matchesHotkey(spec, e)) continue
    // 打字优先只管**裸键**：带 `mod` / `alt` 的组合是打不出来的字，让路只会
    // 把命令面板挡在它的头号使用场景之外（正写着消息时按 Cmd+K）。
    // 裸键要在输入框里也生效（`Esc` 停止生成）必须显式标 `allowInInput`。
    const bare = !spec.mod && !spec.alt
    if (bare && isTypingTarget(el) && !binding.allowInInput) continue
    if (binding.when && !binding.when(e)) continue
    e.preventDefault()
    binding.run(e)
    return true
  }
  return false
}

/** mac 上用 ⌘ 的符号，其余平台写 `Ctrl`——速查表读到的必须是用户屏幕上按得出来的键 */
function isMacPlatform(): boolean {
  return /mac|iphone|ipad/i.test(globalThis.navigator?.userAgent ?? '')
}

/** 键规格 → 用户看得懂的写法（`mod+k` ⇒ `⌘K` / `Ctrl K`） */
export function formatHotkey(spec: string): string {
  const s = parseHotkey(spec)
  const glyph: Record<string, string> = {
    escape: 'Esc',
    enter: 'Enter',
    ' ': 'Space',
    arrowup: '↑',
    arrowdown: '↓',
    arrowleft: '←',
    arrowright: '→',
    digit: '1–9',
  }
  /** 单字符键大写（`k` ⇒ `K`，键盘上那个键帽就是大写）；具名键首字母大写（`delete` ⇒ `Delete`） */
  const named = (k: string) => k.charAt(0).toUpperCase() + k.slice(1)
  const key = glyph[s.key] ?? (s.key.length === 1 ? s.key.toUpperCase() : named(s.key))
  return [s.mod ? (isMacPlatform() ? '⌘' : 'Ctrl') : null, s.alt ? (isMacPlatform() ? '⌥' : 'Alt') : null, key]
    .filter(Boolean)
    .join(isMacPlatform() ? '' : ' ')
}
