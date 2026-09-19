/**
 * VDFS 协议前端侧纯逻辑单测（node 环境）
 *
 * 覆盖：访问位解析、目录判定、扩展名推导、路径代数、校验载荷还原。
 * 这些都是机制级纯函数——页面与渲染器的一切寻址/判定都经它们，必须覆盖。
 */

import { describe, expect, it } from 'vitest'
import {
  actionFileOf,
  isVdfsDir,
  isVdfsDraft,
  isVdfsSystemAddr,
  newFileNameOf,
  parseVdfsValidation,
  vdfsAccessOf,
  vdfsBase,
  vdfsExtOf,
  vdfsJoin,
  vdfsParent,
  type VdfsNode,
} from '../vdfs'
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
