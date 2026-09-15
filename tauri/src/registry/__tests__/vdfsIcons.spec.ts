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
import { getActionIcon, getVdfsIcon, getVdfsIconFor, registerVdfsIcon } from '../vdfsIcons'

const Dummy = defineComponent({ template: '<div />' })

describe('registerVdfsIcon / getVdfsIcon', () => {
  it('未登记的 kind 返回 undefined（走默认图标）', () => {
    expect(getVdfsIcon('unknown-type')).toBeUndefined()
  })

  it('主导航的六类资源均登记了独立图标', () => {
    for (const kind of ['model', 'mcp', 'agent', 'skill', 'session', 'setting']) {
      expect(getVdfsIcon(kind)).toBeTruthy()
    }
  })

  it('可动态登记图标', () => {
    registerVdfsIcon('demo-icon-kind', Dummy)
    expect(getVdfsIcon('demo-icon-kind')).toBe(Dummy)
  })
})

describe('getVdfsIconFor（项级图标分发）', () => {
  it('设置清单的项级图标齐备（自有分区 + 各插件配置目录）', () => {
    // 设置清单 = 自有分区（appearance / about）+ 各插件交出来的配置条目；
    // 条目的项级标识就是**插件目录名**（VdfsWorkbench 的 iconOf 缺省回落节点名）。
    for (const ext of ['appearance', 'about', 'session', 'local', 'web', 'gateway', 'telegram']) {
      expect(getVdfsIconFor({ kind: 'setting', config_type: ext })).toBeTruthy()
    }
  })

  it('config_type 未登记时回退 kind 级图标', () => {
    registerVdfsIcon('demo-kind', Dummy)
    expect(getVdfsIconFor({ kind: 'demo-kind', config_type: 'unknown-ext' })).toBe(Dummy)
  })

  it('非 string 的 config_type 被忽略（节点顶层扩展字段可能是任意类型）', () => {
    expect(getVdfsIconFor({ kind: 'demo-kind', config_type: 42 })).toBe(
      getVdfsIconFor({ kind: 'demo-kind' })
    )
  })

  it('未登记图标的类型返回 undefined', () => {
    expect(getVdfsIconFor({ kind: 'unknown-type' })).toBeUndefined()
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
