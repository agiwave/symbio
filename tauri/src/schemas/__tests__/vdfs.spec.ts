/**
 * VDFS 协议前端侧纯逻辑单测（node 环境）
 *
 * 覆盖：访问位解析、目录判定、扩展名推导、路径代数、校验载荷还原。
 * 这些都是机制级纯函数——页面与渲染器的一切寻址/判定都经它们，必须覆盖。
 */

import { describe, expect, it } from 'vitest'
import {
  VFDS_ROOT,
  actionFileOf,
  isVdfsDir,
  newFileNameOf,
  parseVdfsValidation,
  toVdfsPath,
  toWirePath,
  vdfsAccessOf,
  vdfsBase,
  vdfsExtOf,
  vdfsJoin,
  vdfsMountOf,
  vdfsParent,
  mountNavVisible,
  type VdfsMountInfo,
  type VdfsNode,
} from '../vdfs'

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

describe('路径代数（.vdfs 口径）', () => {
  it('vdfsJoin 规整斜杠', () => {
    expect(vdfsJoin(VFDS_ROOT, 'setting')).toBe('.vdfs/setting')
    expect(vdfsJoin('.vdfs/setting', 'local')).toBe('.vdfs/setting/local')
    expect(vdfsJoin('.vdfs/setting/', '/local/')).toBe('.vdfs/setting/local')
    expect(vdfsJoin('.vdfs/setting', '')).toBe('.vdfs/setting')
    expect(vdfsJoin('', 'a')).toBe('.vdfs/a')
  })

  it('vdfsParent 逐级上溯到虚拟根', () => {
    expect(vdfsParent('.vdfs/setting/local')).toBe('.vdfs/setting')
    expect(vdfsParent('.vdfs/setting')).toBe(VFDS_ROOT)
    expect(vdfsParent(VFDS_ROOT)).toBe(VFDS_ROOT)
    expect(vdfsParent('.vdfs/setting/local/')).toBe('.vdfs/setting')
  })

  it('vdfsBase 取末段名', () => {
    expect(vdfsBase('.vdfs/setting/local')).toBe('local')
    expect(vdfsBase('.vdfs/setting/')).toBe('setting')
    expect(vdfsBase(VFDS_ROOT)).toBe(VFDS_ROOT)
  })

  it('vdfsMountOf 取首段（虚拟根为空）', () => {
    expect(vdfsMountOf('.vdfs/setting/local')).toBe('setting')
    expect(vdfsMountOf('.vdfs/setting')).toBe('setting')
    expect(vdfsMountOf(VFDS_ROOT)).toBe('')
  })
})

describe('口径翻译（前端 .vdfs ↔ 线路 /）', () => {
  it('toWirePath：.vdfs 口径 → 线路口径', () => {
    expect(toWirePath(VFDS_ROOT)).toBe('/')
    expect(toWirePath('.vdfs/session')).toBe('/session')
    expect(toWirePath('.vdfs/session/a')).toBe('/session/a')
    expect(toWirePath('/already/wire')).toBe('/already/wire')
  })

  it('toVdfsPath：线路口径 → .vdfs 口径，且幂等', () => {
    expect(toVdfsPath('/')).toBe(VFDS_ROOT)
    expect(toVdfsPath('/session')).toBe('.vdfs/session')
    expect(toVdfsPath('/session/a')).toBe('.vdfs/session/a')
    expect(toVdfsPath('.vdfs/session')).toBe('.vdfs/session')
    expect(toVdfsPath(toVdfsPath('/session'))).toBe('.vdfs/session')
  })

  it('往返一致', () => {
    for (const p of ['.vdfs', '.vdfs/session', '.vdfs/setting/appearance']) {
      expect(toVdfsPath(toWirePath(p))).toBe(p)
    }
  })
})

describe('mountNavVisible（机制层导航可见性）', () => {
  function mount(partial: Partial<VdfsMountInfo> & { mount: string }): VdfsMountInfo {
    return {
      label: partial.mount,
      order: 0,
      access: 'lt',
      status: 'active',
      root: `/${partial.mount}`,
      ...partial,
    }
  }

  it('缺省可见：字段缺失即视为可见', () => {
    expect(mountNavVisible(mount({ mount: 'session' }))).toBe(true)
    expect(mountNavVisible(mount({ mount: 'session', nav_visible: true }))).toBe(true)
  })

  it('声明不可见即为 false（后端机制层决定，前端不按挂载名特判）', () => {
    expect(mountNavVisible(mount({ mount: 'local', nav_visible: false }))).toBe(false)
    // 空值按缺省可见处理，不在消费点抛错
    expect(mountNavVisible(null)).toBe(true)
  })

  it('按标记过滤：隐藏项不进导航，其余原样保留', () => {
    const mounts = [
      mount({ mount: 'session' }),
      mount({ mount: 'local', nav_visible: false }),
      mount({ mount: 'setting' }),
    ]
    expect(mounts.filter(mountNavVisible).map((m) => m.mount)).toEqual(['session', 'setting'])
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
