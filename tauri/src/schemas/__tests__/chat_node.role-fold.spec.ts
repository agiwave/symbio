/**
 * 角色折叠的判据钉死（`messageRoleOf` / `normalizePrincipal` /
 * `roleOfPrincipal`）——镜像后端 `ChatNode::node_role()` 的折叠与 `Principal` 的命名空间表。
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

describe('normalizePrincipal 的命名空间表（与 Rust 的 namespace 派生逐段对齐）', () => {
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

describe('messageRoleOf：身份只折叠主体（D-147）', () => {
  it('principal.kind 四族一一对应', () => {
    expect(messageRoleOf({ principal: { kind: 'person', id: 'person/local' } })).toBe(CHAT_ROLE_USER)
    expect(messageRoleOf({ principal: { kind: 'agent', id: 'session/main' } })).toBe(
      CHAT_ROLE_ASSISTANT,
    )
    expect(messageRoleOf({ principal: { kind: 'tool', id: 'vdfs/write' } })).toBe(CHAT_ROLE_TOOL)
    expect(messageRoleOf({ principal: { kind: 'system', id: 'system/compactor' } })).toBe(
      CHAT_ROLE_SYSTEM,
    )
  })

  it('地址字符串仍能归一（只是不再收旧形态）：`<ns>/<name>` 由 namespace 派族', () => {
    expect(roleOfPrincipal('person/local')).toBe(CHAT_ROLE_USER)
    expect(roleOfPrincipal('session/main')).toBe(CHAT_ROLE_ASSISTANT)
    expect(roleOfPrincipal('vdfs/write')).toBe(CHAT_ROLE_TOOL)
    expect(roleOfPrincipal('system/compactor')).toBe(CHAT_ROLE_SYSTEM)
  })

  it('旧形态字符串不再归一（SP-008：不为旧数据写迁移分支）', () => {
    expect(normalizePrincipal('agent:main')).toBeUndefined()
    expect(normalizePrincipal('user')).toBeUndefined()
  })

  it('无主体 ⇒ 缺省助手（未标身份的写方按模型产出一族呈现）', () => {
    expect(messageRoleOf({})).toBe(CHAT_ROLE_ASSISTANT)
  })

  it('`role` 不再是身份来源：载荷里混入旧字段也不影响折叠', () => {
    expect(
      messageRoleOf({ principal: { kind: 'tool', id: 'vdfs/write' }, role: 'user' } as never),
    ).toBe(CHAT_ROLE_TOOL)
  })
})

describe('roleOfPrincipal 只折叠不替调用侧做主', () => {
  it('无主体 ⇒ undefined（缺省语义归折叠点）', () => {
    expect(roleOfPrincipal(undefined)).toBeUndefined()
    expect(roleOfPrincipal(null)).toBeUndefined()
  })
})