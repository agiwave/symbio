/**
 * VDFS 列表卡片呈现注册表 — 纯 UI 映射单测（node 环境）
 *
 * 这五条映射原先内联在 `VdfsWorkbench.vue` 里，各自带一段「不许这么改」的注释：
 * 徽标只给目录、状态文案不泄漏后端枚举、不渲染机制级字段。注释钉不住回归，
 * 断言才钉得住——搬进 registry 后这里就是它们的回归网。
 *
 * 钉住的四条约定：
 * 1. **徽标只给目录**：文件（无 `l` 访问位）即使带了 `children` 也不给徽标；
 *    `ext` 是渲染器键，绝不能出现在徽标里。
 * 2. **状态文案只做文案映射**：未知取值返回**空串**（宁可不提示，也不泄漏后端
 *    枚举），且文案不参与任何判据。
 * 3. **不渲染机制级字段**：访问位（`w`）/ `ext` / `path` / `kind` 都不进标签。
 * 4. **脏数据不变成可见元素**：`meta_tags` 里的非字符串与空串丢弃，
 *    `children` 缺省时**不给徽标**（不给好过给一个错的 0）。
 */

import { describe, expect, it } from 'vitest'
import type { VdfsNode } from '@/schemas/vdfs'
import {
  cardBadgeOf,
  cardIconOf,
  cardStatusOf,
  cardStatusTextOf,
  cardTagsOf,
} from '../vdfsCards'

/** 目录 = 访问位含 `l`（可列举），见 `isVdfsDir` */
const DIR = 'rlt'
const FILE = 'rw'

function node(over: Partial<VdfsNode> = {}): VdfsNode {
  return {
    path: '.vdfs/session/x',
    name: 'x',
    title: 'X',
    kind: 'session',
    status: 'active',
    access: FILE,
    ...over,
  }
}

describe('cardStatusOf', () => {
  it('后端状态逐一映射到状态点取值', () => {
    expect(cardStatusOf(node({ status: 'working' }))).toBe('working')
    expect(cardStatusOf(node({ status: 'active' }))).toBe('active')
    expect(cardStatusOf(node({ status: 'disabled' }))).toBe('disabled')
    expect(cardStatusOf(node({ status: 'error' }))).toBe('error')
    expect(cardStatusOf(node({ status: 'warning' }))).toBe('warning')
  })

  it('未知状态回落 muted（画灰点），不是不画', () => {
    expect(cardStatusOf(node({ status: 'whatever' }))).toBe('muted')
    expect(cardStatusOf(node({ status: '' }))).toBe('muted')
  })
})

describe('cardStatusTextOf', () => {
  it('已知状态给中文文案，不是把后端枚举摆出来', () => {
    expect(cardStatusTextOf(node({ status: 'working' }))).toBe('进行中')
    expect(cardStatusTextOf(node({ status: 'active' }))).toBe('就绪')
    expect(cardStatusTextOf(node({ status: 'disabled' }))).toBe('已停用')
    expect(cardStatusTextOf(node({ status: 'error' }))).toBe('出错')
  })

  it('未知状态返回空串：宁可不提示，也不泄漏后端枚举', () => {
    expect(cardStatusTextOf(node({ status: 'vdfs_status_internal' }))).toBe('')
    expect(cardStatusTextOf(node({ status: 'working' }))).not.toBe('working')
  })
})

describe('cardBadgeOf', () => {
  it('目录给子项数', () => {
    expect(cardBadgeOf(node({ access: DIR, children: 12 }))).toBe('12')
    expect(cardBadgeOf(node({ access: DIR, children: 0 }))).toBe('0')
  })

  it('目录但后端未给子项数 → 不给徽标（好过给一个错的 0）', () => {
    expect(cardBadgeOf(node({ access: DIR }))).toBeUndefined()
  })

  it('文件一律不给徽标——即使带了 children', () => {
    expect(cardBadgeOf(node({ access: FILE, children: 3 }))).toBeUndefined()
  })

  it('徽标里绝不会出现 ext（渲染器键属机制细节）', () => {
    const badge = cardBadgeOf(node({ access: DIR, children: 2, ext: 'form' }))
    expect(badge).toBe('2')
    expect(String(badge)).not.toContain('form')
  })
})

describe('cardTagsOf', () => {
  it('meta_tags 原样透传为 muted 标签', () => {
    const tags = cardTagsOf(node({ meta_tags: ['工作目录', '3 条消息'] }))
    expect(tags.map((t) => t.label)).toEqual(['工作目录', '3 条消息'])
    expect(tags.every((t) => t.kind === 'muted')).toBe(true)
  })

  it('脏 meta_tags 不变成可见小方块：非字符串与空串丢弃', () => {
    const tags = cardTagsOf(node({ meta_tags: ['好', '', 42, null, { a: 1 }, '也好'] }))
    expect(tags.map((t) => t.label)).toEqual(['好', '也好'])
  })

  it('meta_tags 非数组时当作没有', () => {
    expect(cardTagsOf(node({ meta_tags: 'not-an-array' }))).toEqual([])
  })

  it('无 updated_at → 无时间标签（不显示「刚刚」这种假时间）', () => {
    expect(cardTagsOf(node({ meta_tags: ['本地'] }))).toHaveLength(1)
  })

  it('有 updated_at 时追加相对时间，排在 meta_tags 之后', () => {
    const tags = cardTagsOf(node({ meta_tags: ['本地'], updated_at: Date.now() }))
    expect(tags).toHaveLength(2)
    expect(tags[0].label).toBe('本地')
    expect(tags[1].label).toBeTruthy()
  })

  it('标签里不含机制字段：访问位 / ext / path / kind 都不出现', () => {
    const tags = cardTagsOf(node({ access: 'rw', ext: 'form', path: '.vdfs/a/b', kind: 'session' }))
    const labels = tags.map((t) => t.label).join('|')
    expect(labels).not.toContain('rw')
    expect(labels).not.toContain('form')
    expect(labels).not.toContain('.vdfs/a/b')
    expect(labels).not.toContain('session')
  })
})

describe('cardIconOf', () => {
  it('目录按目录名查图标', () => {
    expect(cardIconOf(node({ access: DIR, name: 'session' }))).toBeTruthy()
    expect(cardIconOf(node({ access: DIR, name: 'model' }))).toBeTruthy()
  })

  it('目录名未登记 → undefined（由卡片决定不画，而不是画个错的）', () => {
    expect(cardIconOf(node({ access: DIR, name: 'no-such-dir-xyz' }))).toBeUndefined()
  })

  it('文件优先按 config_type（项级扩展名）查图标', () => {
    const withType = cardIconOf(node({ config_type: 'session' }))
    const withoutType = cardIconOf(node({ name: 'session' }))
    expect(withType).toBeTruthy()
    expect(withType).toBe(withoutType)
  })

  it('config_type 是空串 / 非字符串时回落节点名', () => {
    expect(cardIconOf(node({ name: 'model', config_type: '' }))).toBeTruthy()
    expect(cardIconOf(node({ name: 'model', config_type: 42 }))).toBeTruthy()
  })

  it('文件查不到项级图标时回退 kind 级', () => {
    // kind 已登记、config_type 未登记：应当仍能拿到 kind 级图标
    expect(cardIconOf(node({ kind: 'mcp', config_type: 'no-such-type' }))).toBeTruthy()
  })

  it('三级全未命中 → undefined（不画图标，而不是画个错的）', () => {
    expect(cardIconOf(node({ kind: 'no-such-kind', name: 'no-such-name' }))).toBeUndefined()
  })
})
