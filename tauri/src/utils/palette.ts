/**
 * 命令面板（Cmd+K）的**候选计算** —— 纯函数，不含 Vue、不发请求
 *
 * ## 它算的是什么
 *
 * 输入是 `vdfs/tree` 交回的**扁平条目**（地址 + 自述），输出是「按类别分组的可执行项」。
 * 全部判据只来自地址结构与节点自述，**没有任何类别名字面量**——新增一类资源时
 * 后端挂个 provider 就行，这里零改动（与 `VdfsWorkbench` 的「类型只由 `new_type` 决定」
 * 同一条纪律）。
 *
 * ## 为什么分组键是「类别目录的 title」
 *
 * 面板要回答的是「这条结果属于哪一类」，而条目自己只带着地址。类别 = 根的直接子节点
 * （`rel` 恰好一段），它的 title 由 provider 给（「会话」「模型」…），是**产品词**而不是
 * 地址段名——用它当组名，界面就不会出现 `<根>/session` 这种内部地址。
 *
 * ## 为什么不接 `vdfs/search`
 *
 * 后端那条搜索只匹配**文件**（`host::search_via` 显式跳过目录节点），而这里的资源条目
 * 就是目录；它找不到「那一段会话」。`tree` 一次请求给出「地址 + 自述」的扁平清单，
 * 筛词在本地做——于是打字不需要再发请求，面板的开与关也不会放大后端的读放大。
 */

import type { VdfsItem } from '@/schemas/vdfs'

/** 命令到达目的地后的语义：默认导航；只有显式 `new` 才发新建请求 */
export type PaletteAction = 'navigate' | 'new'

/** 面板里的一条可执行项（Enter = 宿主把它 push 成地址页） */
export interface PaletteEntry {
  /** 稳定标识（结果用地址，类别导航用 `nav:<地址>`，新建用 `new:<地址>`） */
  key: string
  /** 主标题 */
  title: string
  /** 次级说明（类别命令 = 浏览/新建动作，资源项 = 地址相对段） */
  hint: string
  /** 目标**数据地址**（浏览器地址由宿主经 `vdfsBrowserPathOf` 换算） */
  addr: string
  /** 缺省 = 导航；新建命令必须显式标 `new`，不能靠 key 前缀猜 */
  action?: PaletteAction
  /** 到达目录后顺带选中的末段名（全局设置等“目录中的一项”用） */
  select?: string
  /** 搜索别名；用户会说“主题”，命令标题却可能叫“外观与提示音” */
  keywords?: string[]
}

/** 一屏内的一组结果 */
export interface PaletteGroup {
  name: string
  entries: PaletteEntry[]
}

export interface PaletteOptions {
  /** 根锚点（用于把全地址切成「相对段」） */
  root: string
  /** 筛选词（原样传入即可，内部做归一化） */
  query: string
  /** 每个类别最多几条（默认 8） */
  perGroup?: number
  /** 整屏最多几条（默认 30） */
  max?: number
  /** 外壳注入的全局命令（设置等不属于 VDFS 新建动作的入口） */
  commands?: PaletteEntry[]
}

/** 「命令」组的组名（打字前后都是它，用户看到的是同一格里长出来的动作） */
export const COMMANDS_GROUP = '命令'

/** 节点相对根的路径段（根自身 = `[]`；非根下的地址 = `null`，不参与面板） */
function relSegments(addr: string, root: string): string[] | null {
  const a = (addr ?? '').trim()
  if (a === root) return []
  if (!a.startsWith(`${root}/`)) return null
  return a.slice(root.length + 1).split('/').filter(Boolean)
}

/** 根的直接子节点 = 类别，它们的下一层 = 资源条目 */
export function splitPaletteNodes(
  nodes: VdfsItem[],
  root: string
): { categories: VdfsItem[]; resources: VdfsItem[] } {
  const categories: VdfsItem[] = []
  const resources: VdfsItem[] = []
  for (const n of nodes ?? []) {
    const segs = relSegments(n.path, root)
    if (segs === null) continue
    if (segs.length === 1) categories.push(n)
    else if (segs.length === 2) resources.push(n)
    // 更深一层不进面板：请求的 `depth` 本来只到条目这一层，
    // 而「在会话里找一条消息」是会话页筛选的事，不是全局跳转的事。
  }
  return { categories, resources }
}

/** 命中强度：3 = 前缀，2 = 标题含，1 = 名称/描述/标签含，null = 不命中 */
function matchScore(item: VdfsItem, q: string): number | null {
  const name = (item.name ?? '').toLowerCase()
  const title = (item.title ?? '').toLowerCase()
  if (title.startsWith(q) || name.startsWith(q)) return 3
  if (title.includes(q)) return 2
  if (name.includes(q)) return 1
  if (item.description?.toLowerCase().includes(q)) return 1
  const tags = item.meta_tags
  if (Array.isArray(tags) && tags.some((t) => typeof t === 'string' && t.toLowerCase().includes(q)))
    return 1
  return null
}

function commandMatches(entry: PaletteEntry, q: string): boolean {
  if (!q) return true
  return [entry.title, entry.hint, ...(entry.keywords ?? [])].some((text) =>
    text.toLowerCase().includes(q)
  )
}

/** 地址的展示用相对段（`session/abc` —— 类别一眼可见，又不重复标题） */
function relHint(addr: string, root: string): string {
  const segs = relSegments(addr, root)
  return segs && segs.length > 1 ? segs.slice(1).join('/') : ''
}

function byScoreThenTitle(a: { score: number; item: VdfsItem }, b: { score: number; item: VdfsItem }) {
  if (a.score !== b.score) return b.score - a.score
  return (a.item.title || a.item.name).localeCompare(b.item.title || b.item.name)
}

/**
 * 算出面板该显示什么。
 *
 * **空词只给命令**：每个根一级类别都能导航，可新建的类别另给一条新建动作；
 * 整库资源条目仍不出现，直到用户输入筛选词。两类命令同去类别目录，但语义不同：
 * 导航只 push，新建在到站后显式发请求（正在浏览某项时也能铺草稿）。
 *
 * 类别导航标题只读类别 `title`，不把内部挂载名摆给用户；新建标题只读
 * `new_type.title`（「添加插件」），hint 用「新建」补足动作语义。
 */
export function buildPalette(nodes: VdfsItem[], opts: PaletteOptions): PaletteGroup[] {
  const root = opts.root ?? ''
  const perGroup = opts.perGroup ?? 8
  const max = opts.max ?? 30
  const q = (opts.query ?? '').trim().toLowerCase()
  const { categories, resources } = splitPaletteNodes(nodes, root)

  const newCommands: PaletteEntry[] = categories.flatMap((c) => {
    const newType = c.new_type
    if (!newType) return []
    const entry: PaletteEntry = {
      key: `new:${c.path}`,
      title: newType.title,
      hint: '新建',
      addr: c.path,
      action: 'new',
      keywords: [`新建${newType.title}`, ...(newType.description ? [newType.description] : [])],
    }
    // 类别名 / 描述仍可命中；`new_type` 自己的标题与「新建…」也参与本地筛选。
    if (q && matchScore(c, q) === null && !commandMatches(entry, q)) return []
    return [entry]
  })
  const navigationCommands: PaletteEntry[] = categories
    .filter((c) => !q || matchScore(c, q) !== null)
    .map((c) => ({
      key: `nav:${c.path}`,
      title: c.title,
      hint: `浏览${c.title}`,
      addr: c.path,
      action: 'navigate' as const,
    }))
  // 新建仍排在导航前：空查询默认高亮的行为不变，Enter 继续执行第一个新建动作。
  const commands = [
    ...(opts.commands ?? []).filter((entry) => commandMatches(entry, q)),
    ...newCommands,
    ...navigationCommands,
  ]

  const groups: PaletteGroup[] = []
  if (q) {
    const byCategory = new Map<string, { score: number; item: VdfsItem }[]>()
    for (const r of resources) {
      const score = matchScore(r, q)
      if (score === null) continue
      const segs = relSegments(r.path, root)
      const dir = segs ? `${root}/${segs[0]}` : ''
      const list = byCategory.get(dir) ?? []
      list.push({ score, item: r })
      byCategory.set(dir, list)
    }
    // 组序 = 类别在根清单里的顺序（左栏看到什么顺序，这里就是什么顺序）
    for (const c of categories) {
      const hits = byCategory.get(c.path)
      if (!hits || hits.length === 0) continue
      hits.sort(byScoreThenTitle)
      groups.push({
        name: c.title || c.name,
        entries: hits.slice(0, perGroup).map(({ item }) => ({
          key: item.path,
          title: item.title || item.name,
          // 组头已经写了类别名，hint 再写一遍是占位不换信息；给地址相对段，
          // 它能把同名的两项区分开（两个类别下都可能有「默认」）。
          hint: relHint(item.path, root),
          addr: item.path,
          action: 'navigate' as const,
        })),
      })
    }
  }
  if (commands.length > 0) groups.unshift({ name: COMMANDS_GROUP, entries: commands })

  // 整屏上限按组序依次给额度：命令组在最前，因此**牺牲的是最靠后的类别**，
  // 而不是把动作砍掉一半（用户打「会」字时至少要看到「新建会话」）。
  let room = max
  const capped: PaletteGroup[] = []
  for (const g of groups) {
    if (room <= 0) break
    const entries = g.entries.slice(0, room)
    room -= entries.length
    if (entries.length > 0) capped.push({ name: g.name, entries })
  }
  return capped
}

/** 分组摊平（键盘上下键走的是这一维，Enter 取它当前那一格） */
export function flattenGroups(groups: PaletteGroup[]): PaletteEntry[] {
  return groups.flatMap((g) => g.entries)
}
