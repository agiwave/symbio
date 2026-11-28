import { describe, expect, it } from 'vitest'
import {
  APPROVAL_SCOPES,
  approvalPolicyBindingOf,
  approvalReasonOf,
  approvalScopesOf,
  isApprovalScope,
} from '../chat_node'

describe('审批范围读侧 schema', () => {
  it('旧卡缺省 / 畸形 scope 都得到空集合，不猜长期授权', () => {
    expect(approvalScopesOf(undefined)).toEqual([])
    expect(approvalScopesOf(null)).toEqual([])
    expect(approvalScopesOf('permanent')).toEqual([])
    expect(approvalScopesOf(['future_scope', 1, null])).toEqual([])
  })

  it('只接收闭集已知词并去重，once 的可用性由消费方保证', () => {
    expect(APPROVAL_SCOPES).toEqual(['once', 'session', 'permanent'])
    expect(approvalScopesOf(['permanent', 'future_scope', 'once', 'permanent'])).toEqual([
      'permanent',
      'once',
    ])
    expect(isApprovalScope('session')).toBe(true)
    expect(isApprovalScope('SESSION')).toBe(false)
  })
})

describe('审批原因读侧 schema', () => {
  it('只接受非空服务端文本，不把畸形值交给渲染器', () => {
    expect(approvalReasonOf('高于当前沙箱上限')).toBe('高于当前沙箱上限')
    expect(approvalReasonOf('   ')).toBeNull()
    expect(approvalReasonOf({ text: '原因' })).toBeNull()
    expect(approvalReasonOf(undefined)).toBeNull()
  })
})

describe('审批策略绑定读侧 schema', () => {
  it('接受服务端绑定；初次 revision 0 合法', () => {
    expect(approvalPolicyBindingOf({ profile_id: 'strict', revision: 0 })).toEqual({
      profile_id: 'strict',
      revision: 0,
    })
  })

  it.each([
    undefined,
    null,
    'strict',
    {},
    { profile_id: '', revision: 0 },
    { profile_id: '   ', revision: 0 },
    { profile_id: 1, revision: 0 },
    { profile_id: 'strict' },
    { profile_id: 'strict', revision: '0' },
    { profile_id: 'strict', revision: -1 },
    { profile_id: 'strict', revision: 1.5 },
    { profile_id: 'strict', revision: Number.MAX_SAFE_INTEGER + 1 },
  ])('缺失或畸形绑定 %# 返回 null，且不产生 default', (value) => {
    expect(approvalPolicyBindingOf(value)).toBeNull()
  })
})
