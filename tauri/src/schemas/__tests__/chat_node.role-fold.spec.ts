/**
 * [T-147d-iii] 角色折叠的判据钉死（`messageRoleOf` / `normalizePrincipal` /
 * `roleOfPrincipal`）——镜像后端 `ChatNode::node_role()` 与 `Principal::from_legacy`。
 */
import { describe, expect, it } from 'vitest'
import {
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_SYSTEM,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  messageRoleOf,
  normalizePrincipal,
  roleOfPrincipal,
} from '@/schemas/chat_node'

describe('normalizePrincipal 的工具命名空间表（与 Rust from_legacy 同一张表）', () => {
  it('合成地址三段都读回 tool 族', () => {
    for (const id of ['legacy/todo_write', 'mcp/mockserv/echo', 'agent/run']) {
      expect(normalizePrincipal(id)?.kind).toBe('tool')
    }
    for (const id of ['vdfs/write', 'web/fetch', 'local/content_search']) {
      expect(normalizePrincipal(id)?.kind).toBe('tool')
    }
  })

  it('session/person/system 段不动原判定', () => {
    expect(normalizePrincipal('session/main')?.kind).toBe('agent')
    expect(normalizePrincipal('person/local')?.kind).toBe('person')
    expect(normalizePrincipal('system/compactor')?.kind).toBe('system')
  })
})

describe('messageRoleOf：格先 kind 后（共存阶段判据）', () => {
  it('role 格在场时优先——即便 principal 是别的族', () => {
    expect(messageRoleOf({ role: CHAT_ROLE_TOOL, principal: { kind: 'agent', id: 'session/main' } })).toBe(
      CHAT_ROLE_TOOL,
    )
  })

  it('role 缺席按 principal.kind 折叠：四族一一对应', () => {
    expect(messageRoleOf({ principal: { kind: 'person', id: 'person/local' } })).toBe(CHAT_ROLE_USER)
    expect(messageRoleOf({ principal: { kind: 'agent', id: 'session/main' } })).toBe(
      CHAT_ROLE_ASSISTANT,
    )
    expect(messageRoleOf({ principal: { kind: 'tool', id: 'vdfs/write' } })).toBe(CHAT_ROLE_TOOL)
    expect(messageRoleOf({ principal: { kind: 'system', id: 'system/compactor' } })).toBe(
      CHAT_ROLE_SYSTEM,
    )
  })

  it('历史裸字符串主体同样折叠（读侧别名一路通吃）', () => {
    expect(messageRoleOf({ principal: 'user' })).toBe(CHAT_ROLE_USER)
    expect(messageRoleOf({ principal: 'agent:main' })).toBe(CHAT_ROLE_ASSISTANT)
    expect(messageRoleOf({ principal: 'tool:vdfs_write' })).toBe(CHAT_ROLE_TOOL)
  })

  it('两格皆无 ⇒ 缺省助手（未标身份的写方按模型产出一族呈现）', () => {
    expect(messageRoleOf({})).toBe(CHAT_ROLE_ASSISTANT)
  })
})

describe('roleOfPrincipal 只折叠不替调用侧做主', () => {
  it('无主体 ⇒ undefined（缺省语义归折叠点）', () => {
    expect(roleOfPrincipal(undefined)).toBeUndefined()
    expect(roleOfPrincipal(null)).toBeUndefined()
  })
})