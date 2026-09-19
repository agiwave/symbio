/**
 * VDFS 地址换算 — 「浏览器地址 ↔ 数据地址」单测（node 环境）
 *
 * 数据地址（`.vdfs/<rel>`）与浏览器地址（`/vdfs/<rel>`）是两个概念，换算是
 * 路由宿主（`views/VdfsView.vue`）唯一的数据职责。它原先内联在宿主里——那里
 * 依赖 vue-router / store / 子组件，规则测不到，只能靠注释守。抽到
 * `schemas/vdfsAddress` 后这里钉住它。
 *
 * 钉住的三条：
 * 1. `:dir(.*)*` 的三种形态（多级数组 / 单级字符串 / 缺省空串）都能收口成
 *    「相对路径」，**空段不产生空路径段**（`['session','']` ≠ `.vdfs/session/`）。
 * 2. 非 VDFS 地址**一律兜底 `/vdfs`**：拼错的地址会静默打开一个空页面，
 *    宁可回首页也不能拼进 URL。前缀陷阱（`.vdfsXXX`）同样落到兜底。
 * 3. 深页面判定只看 `/vdfs/` 前缀：`/vdfs` 是首页（显示 logo），
 *    `/vdfs/<something>` 才是 push 出来的地址页（左上角显示返回键）。
 */

import { describe, expect, it } from 'vitest'
import { isVdfsDeepPage, vdfsAddrOf, vdfsBrowserPathOf, VDFS_HOME_PATH } from '../vdfsAddress'
import { VDFS_ROOT } from '../vdfs'

describe('vdfsAddrOf — 路由参数 → 数据地址', () => {
  it('缺省 / 空串 → 根', () => {
    expect(vdfsAddrOf(undefined)).toBe(VDFS_ROOT)
    expect(vdfsAddrOf('')).toBe(VDFS_ROOT)
    expect(vdfsAddrOf(null)).toBe(VDFS_ROOT)
    expect(vdfsAddrOf([])).toBe(VDFS_ROOT)
  })

  it('单级字符串', () => {
    expect(vdfsAddrOf('session')).toBe('.vdfs/session')
  })

  it('多级数组（:dir(.*)* 的实际形态）', () => {
    expect(vdfsAddrOf(['session', 'abc123'])).toBe('.vdfs/session/abc123')
    expect(vdfsAddrOf(['session', 'abc123', '工作目录'])).toBe('.vdfs/session/abc123/工作目录')
  })

  it('空段不产生空路径段（尾部空串被丢弃）', () => {
    expect(vdfsAddrOf(['session', ''])).toBe('.vdfs/session')
    expect(vdfsAddrOf(['', 'session'])).toBe('.vdfs/session')
  })

  it('空白串视为缺省', () => {
    expect(vdfsAddrOf('   ')).toBe(VDFS_ROOT)
  })
})

describe('vdfsBrowserPathOf — 数据地址 → 浏览器地址', () => {
  it('根 → /vdfs', () => {
    expect(vdfsBrowserPathOf(VDFS_ROOT)).toBe('/vdfs')
  })

  it('根之下 → /vdfs/<rel>（多级原样保留）', () => {
    expect(vdfsBrowserPathOf('.vdfs/session')).toBe('/vdfs/session')
    expect(vdfsBrowserPathOf('.vdfs/session/abc123/工作目录')).toBe('/vdfs/session/abc123/工作目录')
  })

  it('非 VDFS 地址一律兜底首页（不把外来地址拼进 URL）', () => {
    expect(vdfsBrowserPathOf('some/other/place')).toBe('/vdfs')
    expect(vdfsBrowserPathOf('')).toBe('/vdfs')
    expect(vdfsBrowserPathOf('/etc/passwd')).toBe('/vdfs')
  })

  it('前缀陷阱：.vdfsXXX 不算在 .vdfs 之下', () => {
    expect(vdfsBrowserPathOf('.vdfsomething')).toBe('/vdfs')
    expect(vdfsBrowserPathOf('.vdfs2/session')).toBe('/vdfs')
  })

  it('往返一致：地址 → 路径 → 地址', () => {
    for (const rel of ['session', 'session/abc123', 'a/b/c']) {
      const addr = vdfsAddrOf(rel)
      const path = vdfsBrowserPathOf(addr)
      expect(path).toBe(`${VDFS_HOME_PATH}/${rel}`)
      expect(vdfsAddrOf(path.slice(VDFS_HOME_PATH.length + 1))).toBe(addr)
    }
  })
})

describe('isVdfsDeepPage — 是否 push 出来的地址页', () => {
  it('/vdfs 是首页，不是深页面', () => {
    expect(isVdfsDeepPage('/vdfs')).toBe(false)
  })

  it('/vdfs/<something> 是深页面', () => {
    expect(isVdfsDeepPage('/vdfs/session')).toBe(true)
    expect(isVdfsDeepPage('/vdfs/session/abc123')).toBe(true)
  })

  it('其他路由都不是深页面', () => {
    expect(isVdfsDeepPage('/')).toBe(false)
    expect(isVdfsDeepPage('/settings')).toBe(false)
    expect(isVdfsDeepPage('/vdfsx/session')).toBe(false)
  })
})
