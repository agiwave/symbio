/**
 * 命令面板候选计算单测（纯函数，node 环境）
 *
 * 钉住的是四条**会变成静默错误**的判据：
 * - 空词不给整库清单（否则面板一打开是一屏噪音，而用户要的是入口）；
 * - 组名与「能不能新建」都来自节点自述（`title` / `new_type`），不来自类别字面量
 *   ——写死类别名的版本在后端改名时不会报错，只会让某一类永远不出现在面板里；
 * - 命中强度决定顺序（打「abc」时叫「abc 报表」的该排在「登录问题」前面）；
 * - 上限是**按组序依次给额度**：牺牲的是最后的类别，不是动作。
 */
import { describe, expect, it } from 'vitest'
import type { VdfsItem } from '@/schemas/vdfs'
import { buildPalette, flattenGroups, splitPaletteNodes, type PaletteEntry } from '../palette'

const ROOT = '@vfs'

function node(over: Partial<VdfsItem> & { path: string; name: string }): VdfsItem {
  return {
    title: over.name,
    kind: 'dir',
    status: 'active',
    access: 'lt',
    ...over,
  } as VdfsItem
}

const SESSIONS = node({
  path: `${ROOT}/session`,
  name: 'session',
  title: '会话',
  new_type: { ext: 'session', title: '会话' },
})
const MODELS = node({
  path: `${ROOT}/model`,
  name: 'model',
  title: '模型',
  new_type: { ext: 'model', title: '模型' },
})
/** 类别标题与新建类型标题故意不同：不能把「插件管理」当成一种可新建的东西。 */
const PLUGIN_MANAGER = node({
  path: `${ROOT}/plugin_manager`,
  name: 'plugin_manager',
  title: '插件管理',
  new_type: { ext: 'market', title: '添加插件', description: '从插件市场选择' },
})
/** 没有 `new_type` 的类别：给导航，不出现在「新建」动作里 */
const SKILLS = node({ path: `${ROOT}/skill`, name: 'skill', title: '技能' })

const S_ABC = node({ path: `${ROOT}/session/abc`, name: 'abc', title: 'abc 报表' })
const S_LOGIN = node({ path: `${ROOT}/session/s1`, name: 's1', title: '登录问题', description: 'abc 组件的登录' })
const M_GPT = node({ path: `${ROOT}/model/gpt`, name: 'gpt', title: 'GPT-4o', description: '模型网关' })
/** 会话内部的一条消息：比条目更深一层，不该进全局面板 */
const MSG = node({ path: `${ROOT}/session/abc/message/m1`, name: 'm1', title: 'm1 消息' })
/** 外来地址（不在根之下）：一律不参与 */
const FOREIGN = node({ path: `/etc/hosts`, name: 'hosts', title: 'hosts' })

const ALL: VdfsItem[] = [SESSIONS, MODELS, PLUGIN_MANAGER, SKILLS, S_ABC, S_LOGIN, M_GPT, MSG, FOREIGN]

const APPEARANCE: PaletteEntry = {
  key: 'settings:appearance',
  title: '外观与提示音',
  hint: '主题、字体大小、会话结束提示音',
  keywords: ['外观', '主题', '字体', '提示音', '声音', '设置'],
  addr: `${ROOT}/plugin_manager`,
  select: 'appearance',
}

function palette(query: string, opts: { perGroup?: number; max?: number; commands?: PaletteEntry[] } = {}) {
  return buildPalette(ALL, { root: ROOT, query, ...opts })
}

describe('splitPaletteNodes：层别由地址段数决定', () => {
  it('根的直接子节点 = 类别，再下一层 = 资源条目；更深与外来都不参与', () => {
    const { categories, resources } = splitPaletteNodes(ALL, ROOT)
    expect(categories.map((c) => c.path)).toEqual([
      `${ROOT}/session`,
      `${ROOT}/model`,
      `${ROOT}/plugin_manager`,
      `${ROOT}/skill`,
    ])
    expect(resources.map((r) => r.path)).toEqual([
      `${ROOT}/session/abc`,
      `${ROOT}/session/s1`,
      `${ROOT}/model/gpt`,
    ])
  })
})

describe('buildPalette：空词', () => {
  it('★ 只给「命令」组：新建动作在前，所有根一级类别都能导航', () => {
    const groups = palette('')
    expect(groups.map((g) => g.name)).toEqual(['命令'])
    expect(groups[0].entries.map((e) => e.title)).toEqual([
      '会话',
      '模型',
      '添加插件',
      '会话',
      '模型',
      '插件管理',
      '技能',
    ])
    expect(groups[0].entries.map((e) => e.action)).toEqual([
      'new',
      'new',
      'new',
      'navigate',
      'navigate',
      'navigate',
      'navigate',
    ])
    expect(groups[0].entries.slice(0, 3).map((e) => e.addr)).toEqual([
      `${ROOT}/session`,
      `${ROOT}/model`,
      `${ROOT}/plugin_manager`,
    ])
  })

  it('新建标题只读 new_type.title，导航标题只读类别 title，不泄漏内部 name', () => {
    const entries = flattenGroups(palette(''))
    const add = entries.find((e) => e.key === `new:${ROOT}/plugin_manager`)
    const manage = entries.find((e) => e.key === `nav:${ROOT}/plugin_manager`)
    expect(add).toMatchObject({ title: '添加插件', hint: '新建', action: 'new' })
    expect(manage).toMatchObject({ title: '插件管理', hint: '浏览插件管理', action: 'navigate' })
    expect(`${add?.title} ${add?.hint} ${manage?.title} ${manage?.hint}`).not.toContain('plugin_manager')
  })

  it('外壳注入的全局命令排在新建动作前，空查询也可见', () => {
    const groups = palette('', { commands: [APPEARANCE] })
    expect(groups[0].entries.map((e) => e.title)).toEqual([
      '外观与提示音',
      '会话',
      '模型',
      '添加插件',
      '会话',
      '模型',
      '插件管理',
      '技能',
    ])
    expect(groups[0].entries[0].select).toBe('appearance')
  })

  it('没有可新建的类别仍给导航；连类别都没有才是空面板', () => {
    const groups = buildPalette([SKILLS], { root: ROOT, query: '' })
    expect(flattenGroups(groups)).toEqual([
      expect.objectContaining({ title: '技能', action: 'navigate', addr: `${ROOT}/skill` }),
    ])
    expect(buildPalette([], { root: ROOT, query: '' })).toEqual([])
  })
})

describe('buildPalette：打字之后', () => {
  it('★ 命中强度排序：标题前缀在描述包含之前，组序跟着左栏序', () => {
    const groups = palette('abc')
    expect(groups.map((g) => g.name)).toEqual(['会话'])
    expect(groups[0].entries.map((e) => e.title)).toEqual(['abc 报表', '登录问题'])
  })

  it('类别名命中 ⇒ 命令组给新建与导航，资源仍按类别另组', () => {
    const groups = palette('模型')
    expect(groups.map((g) => g.name)).toEqual(['命令', '模型'])
    expect(groups[0].entries.map((e) => [e.title, e.action])).toEqual([
      ['模型', 'new'],
      ['模型', 'navigate'],
    ])
    expect(groups[1].entries.map((e) => e.title)).toEqual(['GPT-4o'])
  })

  it('不可新建类别只给导航；new_type.title 与内部 name 也都可搜索', () => {
    expect(flattenGroups(palette('技能'))).toEqual([
      expect.objectContaining({ title: '技能', action: 'navigate' }),
    ])
    expect(flattenGroups(palette('添加')).map((e) => [e.title, e.action])).toEqual([
      ['添加插件', 'new'],
    ])
    expect(flattenGroups(palette('新建会话')).map((e) => [e.title, e.action])).toEqual([
      ['会话', 'new'],
    ])
    expect(flattenGroups(palette('插件')).map((e) => [e.title, e.action])).toEqual([
      ['添加插件', 'new'],
      ['插件管理', 'navigate'],
    ])
    const byInternalName = flattenGroups(palette('plugin_manager'))
    expect(byInternalName.map((e) => e.action)).toEqual(['new', 'navigate'])
    expect(byInternalName.map((e) => e.title).join('|')).not.toContain('plugin_manager')
  })

  it('大小写不敏感（中文没有大小写，英文条目却是常态）', () => {
    expect(palette('GPT').map((g) => g.name)).toEqual(['模型'])
    expect(palette(' gpt ').map((g) => g.name)).toEqual(['模型'])
  })

  it('★ 资源项的 hint 是地址相对段，不是组名（组头已经说过类别，重复一遍不换信息）', () => {
    const groups = palette('abc')
    expect(groups[0].entries.map((e) => e.hint)).toEqual(['abc', 's1'])
  })

  it('全局设置命令可用用户词命中（主题 / 字体 / 提示音）', () => {
    for (const query of ['外观', '主题', '字体', '提示音', '声音']) {
      const groups = palette(query, { commands: [APPEARANCE] })
      expect(groups[0]?.name).toBe('命令')
      expect(groups[0]?.entries[0]?.key).toBe(APPEARANCE.key)
      expect(groups[0]?.entries[0]?.action).not.toBe('new')
    }
  })

  it('一个都没命中 ⇒ 空数组，由组件去区分「没有匹配」与「还没打字」', () => {
    expect(palette('不存在的东西')).toEqual([])
  })
})

describe('buildPalette：额度', () => {
  const many: VdfsItem[] = [
    SESSIONS,
    MODELS,
    ...Array.from({ length: 12 }, (_, i) => node({ path: `${ROOT}/session/x${i}`, name: `x${i}` })),
    ...Array.from({ length: 12 }, (_, i) => node({ path: `${ROOT}/model/y${i}`, name: `y${i}` })),
  ]

  it('每组最多 perGroup 条', () => {
    const groups = buildPalette(many, { root: ROOT, query: 'x', perGroup: 5 })
    expect(groups).toHaveLength(1)
    expect(groups[0].entries).toHaveLength(5)
  })

  it('★ 整屏上限先给动作、再按组序给额度：牺牲的是最后一个类别而不是动作', () => {
    const groups = buildPalette(many, { root: ROOT, query: '', perGroup: 8, max: 3 })
    expect(groups.map((g) => g.name)).toEqual(['命令'])
    expect(groups[0].entries).toHaveLength(3)
    expect(groups[0].entries.slice(0, 2).map((e) => e.action)).toEqual(['new', 'new'])

    const typed = buildPalette(many, { root: ROOT, query: 'y', perGroup: 8, max: 2 })
    expect(flattenGroups(typed)).toHaveLength(2)
  })
})
