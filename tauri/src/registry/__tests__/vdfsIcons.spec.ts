/**
 * VDFS 图标注册表 — 前端纯 UI 映射单测（node 环境）
 *
 * 注册表只登记两件事：
 * - kind（或 kind:ext）→ 图标组件
 * - 动作 icon 名 / 动作 id → SVG path
 *
 * 资源的**存在 / 能力 / 寻址 / 顺序 / 标签**一律来自后端（VDFS 挂载点与节点），
 * 前端不硬编码类型清单；详情组件的装配在 `registry/vdfsRenderers.ts`，不在此断言。
 */
import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import { getActionIcon, iconForNode, registerVdfsIcon } from '../vdfsIcons'

const Dummy = defineComponent({ template: '<div />' })

/**
 * 图标登记与查找。
 *
 * ⚠️ 查找一律走 `iconForNode`（**唯一实现**）。从前还有 `getVdfsIcon(kind)` /
 * `getVdfsIconFor({kind, config_type})` 两个中间层，它们各自带一条「回退到
 * `icons[kind]`」的尾巴——那条尾巴会在第一步就用笼统的 kind（挂载点的 `dir`）
 * 把名字查询挡死。两个辅助函数已删除，**别再引入**：多一个入口就多一处能写错
 * 回退顺序的地方，而顺序正是本次事故的成因。
 */
describe('registerVdfsIcon / iconForNode（登记与查找）', () => {
  it('未登记的 kind 返回 undefined（走默认图标）', () => {
    expect(iconForNode({ kind: 'unknown-type' })).toBeUndefined()
  })

  it('主导航的六类资源均登记了独立图标', () => {
    for (const kind of ['model', 'mcp', 'agent', 'skill', 'session', 'plugin_manager']) {
      expect(iconForNode({ kind }), `${kind} 缺图标`).toBeTruthy()
    }
  })

  it('会话内部各区段的 kind 均有独立图标（不然侧栏四个文件夹长得一样）', () => {
    // 后端声明的协议词：messages / inbox 在 `plugin/words.rs`，
    // subsession / dir 在 `workdir/mod.rs`。图标缺失只会退成文件夹兜底，
    // 不会有任何测试变红——因此这里逐个钉住。
    const kinds = ['messages', 'inbox', 'subsession', 'dir']
    for (const kind of kinds) {
      expect(iconForNode({ kind }), `${kind} 缺图标`).toBeTruthy()
    }
    // 互不相同：同一张图出现在四个区段上，等于没有导航
    const seen = new Set(kinds.map((kind) => iconForNode({ kind })))
    expect(seen.size).toBe(kinds.length)
  })

  it('可动态登记图标', () => {
    registerVdfsIcon('demo-icon-kind', Dummy)
    expect(iconForNode({ kind: 'demo-icon-kind' })).toBe(Dummy)
  })
})

describe('iconForNode 的项级分发（kind:config_type）', () => {
  it('插件管理清单的项级图标齐备（自有分区 + 各插件配置目录）', () => {
    // 清单 = 自有分区（appearance / about）+ 各插件的条目；
    // 条目的项级标识就是**插件目录名**（config_type 携带）。
    for (const ext of ['appearance', 'about', 'session', 'local', 'web', 'gateway', 'telegram']) {
      expect(
        iconForNode({ kind: 'plugin_manager', name: ext, config_type: ext }),
        `${ext} 项级图标缺失`,
      ).toBeTruthy()
    }
  })

  it('项级键（kind:ext）优先于配置键与名字', () => {
    registerVdfsIcon('demo-kind', svgStub())
    registerVdfsIcon('demo-kind:demo-ext', Dummy)
    expect(iconForNode({ kind: 'demo-kind', name: 'demo-ext', config_type: 'demo-ext' })).toBe(Dummy)
  })

  it('非 string 的 config_type 被忽略（节点顶层扩展字段可能是任意类型）', () => {
    registerVdfsIcon('demo-kind-2', Dummy)
    const icons = [
      iconForNode({ kind: 'demo-kind-2', name: 'x', config_type: 42 }),
      iconForNode({ kind: 'demo-kind-2', name: 'x', config_type: '' }),
      iconForNode({ kind: 'demo-kind-2', name: 'x' }),
    ]
    expect(icons[0], '非 string 不得被当作配置键').toBe(Dummy)
    expect(icons[1], '空串不得被当作配置键').toBe(Dummy)
    expect(new Set(icons).size, '三种写法必须落同一张图').toBe(1)
  })

  it('未登记图标的类型返回 undefined', () => {
    expect(iconForNode({ kind: 'unknown-type' })).toBeUndefined()
  })
})

/** 造一个与 Dummy 不同的图标组件（用于断言「优先命中哪一个」） */
function svgStub() {
  return defineComponent({ template: '<svg />' })
}

/**
 * `iconForNode` —— **取图标这件事的唯一实现**。
 *
 * 这组用例锁定的是**一个真实缺陷**，不是形式检查。
 *
 * ## 事故经过
 *
 * `<根>` 的挂载点节点由后端 `composite/vdfs.rs::dir_node` 用 `VdfsNode::dir(...)`
 * 产出 —— 它的 `kind` 恒为 **`"dir"`**（`VDFS_KIND_DIR`），且**不带 `config_type`**；
 * 身份只有 name / title / description / access。而挂载点图标是登记在**裸名字**上的
 * （`session` / `model` / `agent` / …）。
 *
 * 侧栏原先写的是「只按名字查一级」，能命中；卡片写的是「项级 → kind → 名字」三级。
 * 我为了「kind 优先」把侧栏第一跳改成 `kind`，却没保留**名字兜底**——`kind="dir"`
 * 查不到、`config_type` 又没有，于是主界面左栏的「会话 / 模型 / 智能体 / MCP /
 * 技能 / 插件管理」整排退成了同一个默认图标。
 *
 * ## 这组用例为什么必须存在
 *
 * 该缺陷**类型检查抓不到**（`kind` 是 `string`，任何取值都合法），而 `iconForNode`
 * 返回 `undefined` 时调用方只是「不画图标」，不抛错、不报红——**静默退化**。
 * 唯一能钉住它的是把「真实后端形状」喂进来断言。
 *
 * ⚠️ 夹具里的 `kind` **必须是 `"dir"`**，不能图省事写成空串：后端从不发空 kind，
 * 拿一个不存在的形状做夹具，真回归来了照样是绿的。
 */
describe('iconForNode（按节点取图标的唯一实现）', () => {
  /** 后端 `composite/vdfs.rs::dir_node` 的真实形状：`kind = "dir"`、无 `config_type` */
  const ROOT_MOUNTS = [
    { name: 'session', title: '会话' },
    { name: 'model', title: '模型' },
    { name: 'agent', title: '智能体' },
    { name: 'mcp', title: 'MCP' },
    { name: 'skill', title: '技能' },
    { name: 'plugin_manager', title: '插件管理' },
  ]

  it('**主界面六个挂载点各得一张不同的图**（回归：kind=dir 时它们曾整排退成默认图）', () => {
    const icons = ROOT_MOUNTS.map((d) => iconForNode({ name: d.name, kind: 'dir' }))
    const detail = ROOT_MOUNTS.map((d, i) => `${d.name}=${icons[i] ? 'ok' : 'MISSING'}`).join(', ')
    expect(icons.every(Boolean), `有一项取不到图标：${detail}`).toBe(true)
    expect(new Set(icons).size, `图标重复（退成同一个默认图）：${detail}`).toBe(ROOT_MOUNTS.length)
  })

  it('kind 字段整个缺失（undefined）时同样按 name 出图', () => {
    for (const d of ROOT_MOUNTS) {
      expect(iconForNode({ name: d.name }), `${d.name} 取不到图标`).toBeTruthy()
    }
  })

  it('kind 为空串（后端不发，但形状合法）时也按 name 出图', () => {
    for (const d of ROOT_MOUNTS) {
      expect(iconForNode({ name: d.name, kind: '' }), `${d.name} 取不到图标`).toBeTruthy()
    }
  })

  it('项级键命中：plugin_manager 的分区靠 config_type 出图', () => {
    // 分区节点的 name 是插件名（如 telegram），kind 也不是分区名——
    // **只有 config_type 能命中**。只查 name 或只查 kind 都会退成默认图标。
    expect(
      iconForNode({ kind: 'plugin_manager', name: 'telegram', config_type: 'telegram' })
    ).toBeTruthy()
  })

  it('kind 级命中：会话内部区段按协议 kind 出图', () => {
    for (const kind of ['messages', 'inbox', 'subsession', 'dir']) {
      expect(iconForNode({ kind, name: 'whatever' }), `${kind} 取不到图标`).toBeTruthy()
    }
  })

  it('全未命中 → undefined（由调用方决定兜底，而不是在这里随便画一个）', () => {
    expect(iconForNode({ kind: 'no-such-kind', name: 'no-such-name' })).toBeUndefined()
  })

  it('空节点不炸（缺 kind / name 时按 undefined 处理）', () => {
    expect(iconForNode({})).toBeUndefined()
  })
})

describe('getActionIcon（动作图标）', () => {
  it('语义动作 id 自带默认图标', () => {
    for (const id of ['save', 'test', 'delete', 'set-default', 'open-container']) {
      expect(getActionIcon({ id })).toBeTruthy()
    }
  })

  it('icon 名优先于动作 id（区分同 id 多形态，如「跳过校验保存」）', () => {
    expect(getActionIcon({ id: 'save', icon: 'save-skip' })).toBe(
      getActionIcon({ id: 'save-skip' })
    )
  })

  it('无映射 → undefined（渲染器回落为文字按钮）', () => {
    expect(getActionIcon({ id: 'whatever' })).toBeUndefined()
    expect(getActionIcon({ id: 'whatever', icon: 'nope' })).toBeUndefined()
  })
})
