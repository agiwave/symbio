/**
 * VDFS 协议前端侧纯逻辑单测（node 环境）
 *
 * 覆盖：访问位解析、目录判定、扩展名推导、路径代数、校验载荷还原。
 * 这些都是机制级纯函数——页面与渲染器的一切寻址/判定都经它们，必须覆盖。
 */

import { describe, expect, it } from 'vitest'
import {
  VDFS_STATUS_ABORTED,
  VDFS_STATUS_ACTIVE,
  VDFS_STATUS_COMPLETED,
  VDFS_STATUS_FAILED,
  VDFS_STATUS_PENDING,
  VDFS_STATUS_REMOVED,
  VDFS_STATUS_STREAMING,
  VDFS_STATUS_WAITING_USER_ACTION,
  VDFS_STATUS_WORKING,
  actionFileOf,
  isVdfsDir,
  isVdfsDraft,
  isVdfsSystemAddr,
  newFileNameOf,
  parseVdfsValidation,
  sessionRuntimeOf,
  vdfsAccessOf,
  vdfsBase,
  vdfsExtOf,
  vdfsJoin,
  vdfsParent,
  type VdfsNode,
} from '../vdfs'
// 命名空间导入：用于「导出的常量集合恰好是这些」这类**闭集**断言
// （见文件末尾的「变更词汇表」describe）。它不能只靠类型检查表达——
// 多导出一个人家看不见的常量，类型系统不会报错。
import * as vdfsModule from '../vdfs'
import {
  MESSAGE_STATUS_ABORTED,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_FAILED,
  MESSAGE_STATUS_PENDING,
  MESSAGE_STATUS_REMOVED,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
  MESSAGE_STATUSES,
  isMessageStatus,
} from '../chat_message'
import { resetVdfsRoot, setVdfsRoot, vdfsRoot } from '../vdfsRoot'

// 合成根：**故意不是**后端当前挂载名——本文件全部断言与根名无关，
// 后端改挂载名（换成任何值）这里一个字都不用改。
const ROOT = '@vfs'
setVdfsRoot(ROOT)

function node(partial: Partial<VdfsNode> & { name: string }): VdfsNode {
  return {
    path: `/${partial.name}`,
    title: partial.name,
    kind: 'file',
    status: 'active',
    access: 'r',
    ...partial,
  }
}

describe('vdfsAccessOf / isVdfsDir', () => {
  it('按紧凑串解析四个访问位', () => {
    expect(vdfsAccessOf({ access: 'rwlt' })).toEqual({
      read: true, write: true, list: true, traverse: true,
    })
    expect(vdfsAccessOf({ access: 'lt' })).toEqual({
      read: false, write: false, list: true, traverse: true,
    })
    expect(vdfsAccessOf({ access: '' }).read).toBe(false)
    expect(vdfsAccessOf(null).list).toBe(false)
  })

  it('目录判定只看访问位 l，不看 kind（机制不含类型特判）', () => {
    expect(isVdfsDir({ access: 'lt' })).toBe(true)
    expect(isVdfsDir({ access: 'rw' })).toBe(false)
    // kind 声称是目录但无 l 位 → 仍非目录（先落到变量上以绕开字面量多余属性检查）
    const claimsDir = { access: 'r', kind: 'dir' }
    expect(isVdfsDir(claimsDir)).toBe(false)
  })
})

describe('isVdfsSystemAddr（地址空间的两个半边）', () => {
  it('虚拟根自身与根之下都是系统资源地址', () => {
    expect(isVdfsSystemAddr(ROOT)).toBe(true)
    expect(isVdfsSystemAddr(vdfsRoot())).toBe(true)
    expect(isVdfsSystemAddr('@vfs/model/gpt4')).toBe(true)
  })

  it('工作目录里的物理文件不是系统资源地址（重命名只对物理侧开放）', () => {
    expect(isVdfsSystemAddr('notes/a.md')).toBe(false)
    expect(isVdfsSystemAddr('/tmp/demo.zip')).toBe(false)
    // 前缀相近但不是系统根：不得按 startsWith(根) 误判
    expect(isVdfsSystemAddr('@vfsx/a')).toBe(false)
  })
})

describe('isVdfsDraft（草稿 == 没有路径）', () => {
  it('没有 path（含空串 / 缺字段）即为草稿', () => {
    expect(isVdfsDraft({ path: '' })).toBe(true)
    expect(isVdfsDraft({})).toBe(true)
  })

  it('null / undefined 也算草稿（调用方不必先判空）', () => {
    expect(isVdfsDraft(null)).toBe(true)
    expect(isVdfsDraft(undefined)).toBe(true)
  })

  it('一旦落盘（有非空 path）就不是草稿——名字不是判据', () => {
    expect(isVdfsDraft({ path: '@vfs/model/gpt4' })).toBe(false)
    expect(isVdfsDraft({ path: 'notes/a.md' })).toBe(false)
  })
})

describe('vdfsExtOf', () => {
  it('显式 ext 优先于 name 推导', () => {
    expect(vdfsExtOf(node({ name: 'local', ext: 'form' }))).toBe('form')
  })

  it('无显式 ext 时由末段名推导（大小写归一）', () => {
    expect(vdfsExtOf(node({ name: 'note.MD' }))).toBe('md')
    expect(vdfsExtOf(node({ name: '/a/b/config.json' }))).toBe('json')
  })

  it('无扩展名 / 隐藏文件 / 尾点 均不推导', () => {
    expect(vdfsExtOf(node({ name: 'noext' }))).toBeUndefined()
    expect(vdfsExtOf(node({ name: '.env' }))).toBeUndefined()
    expect(vdfsExtOf(node({ name: 'a.' }))).toBeUndefined()
  })
})

describe('路径代数（根锚点口径）', () => {
  it('vdfsJoin 规整斜杠', () => {
    expect(vdfsJoin(ROOT, 'setting')).toBe(`${ROOT}/setting`)
    expect(vdfsJoin('@vfs/setting', 'local')).toBe('@vfs/setting/local')
    expect(vdfsJoin('@vfs/setting/', '/local/')).toBe('@vfs/setting/local')
    expect(vdfsJoin('@vfs/setting', '')).toBe('@vfs/setting')
    expect(vdfsJoin('', 'a')).toBe('@vfs/a')
  })

  it('vdfsParent 逐级上溯到虚拟根', () => {
    expect(vdfsParent('@vfs/setting/local')).toBe('@vfs/setting')
    expect(vdfsParent(`${ROOT}/setting`)).toBe(ROOT)
    expect(vdfsParent(ROOT)).toBe(ROOT)
    expect(vdfsParent('@vfs/setting/local/')).toBe('@vfs/setting')
  })

  it('vdfsBase 取末段名', () => {
    expect(vdfsBase('@vfs/setting/local')).toBe('local')
    expect(vdfsBase('@vfs/setting/')).toBe('setting')
    expect(vdfsBase(ROOT)).toBe(ROOT)
  })
})

describe('newFileNameOf（整包导入的目标名）', () => {
  it('保留原名主干 + 换成类型扩展名', () => {
    expect(newFileNameOf('demo.zip', 'zip')).toBe('demo.zip')
    // 多段扩展名只换最后一段（`pkg.tar.gz` → `pkg.tar.zip`）
    expect(newFileNameOf('pkg.tar.gz', 'zip')).toBe('pkg.tar.zip')
    // 无扩展名 / 类型无 ext
    expect(newFileNameOf('README', 'zip')).toBe('README.zip')
    expect(newFileNameOf('demo.zip', '')).toBe('demo')
  })

  it('隐藏文件（.gitignore）不作主干切割，路径只取末段', () => {
    expect(newFileNameOf('.gitignore', 'zip')).toBe('.gitignore.zip')
    expect(newFileNameOf('C:\\tmp\\dir\\demo.zip', 'zip')).toBe('demo.zip')
  })
})

describe('actionFileOf（动作结果里的文件载荷）', () => {
  it('认形状不认动作：有 filename + b64 就下载', () => {
    expect(actionFileOf({ id: 'b1', filename: 'b1.zip', b64: 'UEsDBA==' })).toEqual({
      filename: 'b1.zip',
      b64: 'UEsDBA==',
    })
  })

  it('非对象 / 缺字段 / 空串一律不算文件载荷', () => {
    expect(actionFileOf(null)).toBeNull()
    expect(actionFileOf(undefined)).toBeNull()
    expect(actionFileOf('b1.zip')).toBeNull()
    expect(actionFileOf({ filename: 'b1.zip' })).toBeNull()
    expect(actionFileOf({ b64: 'UEsDBA==' })).toBeNull()
    expect(actionFileOf({ filename: '', b64: '' })).toBeNull()
    // 「测试连接」这类只回 status 的动作：没有文件载荷
    expect(actionFileOf({ status: 'connected' })).toBeNull()
  })
})

describe('parseVdfsValidation', () => {
  it('从错误文案里还原字段级校验载荷', () => {
    const raw =
      '校验未通过: {"message":"设置校验未通过","fields":[{"field":"port","message":"不能大于 65535"}]}'
    const parsed = parseVdfsValidation(raw)
    expect(parsed?.message).toBe('设置校验未通过')
    expect(parsed?.fields).toEqual([{ field: 'port', message: '不能大于 65535' }])
  })

  it('可接受 Error 对象（服务层抛错形态）', () => {
    const err = new Error('{"message":"非法的键","fields":[]}')
    expect(parseVdfsValidation(err)?.message).toBe('非法的键')
  })

  it('纯文本错误返回 null（调用方按纯文本展示）', () => {
    expect(parseVdfsValidation('端口不合法')).toBeNull()
    expect(parseVdfsValidation('')).toBeNull()
    expect(parseVdfsValidation(undefined)).toBeNull()
  })
})

describe('vdfsRoot — 根锚点', () => {
  it('登记后可读回，且自动归一化尾部分隔符', () => {
    setVdfsRoot(`${ROOT}/`)
    expect(vdfsRoot()).toBe(ROOT)
  })

  it('reset 后退化为空串（无虚拟半）', () => {
    resetVdfsRoot()
    expect(vdfsRoot()).toBe('')
    expect(isVdfsSystemAddr('anything')).toBe(false)
    setVdfsRoot(ROOT) // 恢复，避免影响其它用例（若拆分文件可删）
  })
})

/**
 * 消息类节点状态词**只有一份定义**（在 `chat_message`），VDFS 侧是别名。
 *
 * 这条断言锁的是一个具体的失败模式：状态词表曾有两份，`aborted` 只补进了其中
 * 一份，消费端 `transcriptStream`（当时叫 `vdfsTranscriptSync`）于是把整条状态
 * **静默丢弃**——中止后的 Turn 显示为已完成、重试入口不出现。两份变一份后，
 * "漏改一份"在结构上不再可能；
 * 但"有人又写了一行字面量"仍可能，所以这里把它钉住。
 */
describe('节点状态词：VDFS 侧是 chat_message 的别名', () => {
  it('消息类 VDFS 常量与消息状态常量**同值**（不是各写一份字面量）', () => {
    expect(VDFS_STATUS_PENDING).toBe(MESSAGE_STATUS_PENDING)
    expect(VDFS_STATUS_STREAMING).toBe(MESSAGE_STATUS_STREAMING)
    expect(VDFS_STATUS_WAITING_USER_ACTION).toBe(MESSAGE_STATUS_WAITING_USER_ACTION)
    expect(VDFS_STATUS_COMPLETED).toBe(MESSAGE_STATUS_COMPLETED)
    expect(VDFS_STATUS_FAILED).toBe(MESSAGE_STATUS_FAILED)
    expect(VDFS_STATUS_ABORTED).toBe(MESSAGE_STATUS_ABORTED)
    expect(VDFS_STATUS_REMOVED).toBe(MESSAGE_STATUS_REMOVED)
  })

  it('VDFS 侧可达的消息状态词集合与词表**完全相等**（漏一个或凭空多一个都红）', () => {
    const viaVdfs = [
      VDFS_STATUS_PENDING,
      VDFS_STATUS_STREAMING,
      VDFS_STATUS_WAITING_USER_ACTION,
      VDFS_STATUS_COMPLETED,
      VDFS_STATUS_FAILED,
      VDFS_STATUS_ABORTED,
      VDFS_STATUS_REMOVED,
    ]
    expect([...viaVdfs].sort()).toEqual([...MESSAGE_STATUSES].sort())
  })

  it('会话类状态与未知词不属于消息词表（isMessageStatus 拦住它们）', () => {
    expect(isMessageStatus(VDFS_STATUS_WORKING)).toBe(false)
    expect(isMessageStatus(VDFS_STATUS_ACTIVE)).toBe(false)
    expect(isMessageStatus('paused')).toBe(false)
  })
})

/**
 * `sessionRuntimeOf`：会话节点 → 运行态。
 *
 * 后端把运行态当作**节点属性**下发（`SessionStateChange` 写 `attributes.outcome`
 * / `error` / `warning`），前端必须**逐个读出来**。漏读一个的代价不是"少显示一条"，
 * 而是整条状态被**静默丢弃**——UI 永远看不到它，且不报错（`warning` 恰好就被漏过：
 * 后端一直在写，`SessionRuntime` 里没有这个字段，类型检查才发现）。
 */
describe('sessionRuntimeOf：三个场景属性都必须落地', () => {
  it('状态直通；无场景属性时只有 status', () => {
    expect(sessionRuntimeOf(node({ name: 's', status: 'active' }))).toEqual({ status: 'active' })
  })

  it('outcome 只在三取值内落地（未知值不推断）', () => {
    expect(sessionRuntimeOf({ ...node({ name: 's' }), outcome: 'aborted' }).outcome).toBe('aborted')
    expect(sessionRuntimeOf({ ...node({ name: 's' }), outcome: 'failed' }).outcome).toBe('failed')
    expect(sessionRuntimeOf({ ...node({ name: 's' }), outcome: 'weird' }).outcome).toBeUndefined()
  })

  it('error 与 warning 各自独立落地（错误 ≠ 告警）', () => {
    const failed = sessionRuntimeOf({
      ...node({ name: 's', status: 'failed' }),
      error: '能力收集失败',
    })
    expect(failed.error).toBe('能力收集失败')
    expect(failed.warning).toBeUndefined()

    const warned = sessionRuntimeOf({
      ...node({ name: 's', status: 'working' }),
      warning: '消息持久化失败（消息仍在内存中）',
    })
    expect(warned.warning).toBe('消息持久化失败（消息仍在内存中）')
    expect(warned.error).toBeUndefined()
  })

  it('空串视为缺省（不得让一个空告警占住"有待处理状态"的判定）', () => {
    const rt = sessionRuntimeOf({ ...node({ name: 's' }), error: '', warning: '' })
    expect(rt.error).toBeUndefined()
    expect(rt.warning).toBeUndefined()
  })
})

describe('变更词汇表：**闭集**，恰好三个取值', () => {
  /**
   * 这条断言锁的是**判据**本身，不是某个具体取值：
   *
   * > 一个变更取值（或一个载荷字段）必须有**生产性生产者**，否则它不是词汇的
   * > 一部分，只是别人误以为它存在的理由。
   *
   * 曾经这里有 6 个：`renamed` / `appended` / `truncated` 三个取值没有任何真实
   * 生产者（源自「消息寄生 VDFS」时代），却足以让消费端写出永远不执行的
   * `if (change.delta)`、并让「这条通道会不会给我正文」变成要读实现才能回答的问题。
   * 它们已删除——本断言保证下一个想加回来的人**必须同时给出生产者**，否则红。
   */
  it('导出的 VDFS_CHANGE_* 恰好是 created / updated / deleted 三个', () => {
    const names = Object.keys(vdfsModule)
      .filter((k) => k.startsWith('VDFS_CHANGE_'))
      .sort()
    expect(names).toEqual(['VDFS_CHANGE_CREATED', 'VDFS_CHANGE_DELETED', 'VDFS_CHANGE_UPDATED'])
    // 三个取值的**字面量**也要锁：改了值就是改协议（后端有同名常量，跨栈由
    // scripts/protocol-mirror-audit.mjs 校验）
    expect(
      names.map((n) => (vdfsModule as unknown as Record<string, string>)[n]).sort(),
    ).toEqual(['created', 'deleted', 'updated'])
  })

  it('重同步指令与变更取值**不是一类**：它是指令，且刻意不带 path', () => {
    // 若把 RESYNC 也算进 VDFS_CHANGE_* 前缀，上面的闭集断言会红——
    // 这条测试反过来锁住「它没被并进去」。
    expect(vdfsModule.VDFS_BUS_RESYNC).toBe('resync')
    expect(vdfsModule.VDFS_BUS_RESYNC.startsWith('VDFS_CHANGE_')).toBe(false)
  })
})
