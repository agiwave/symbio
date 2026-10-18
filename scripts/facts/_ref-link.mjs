#!/usr/bin/env node
/**
 * facts/_ref-link — 生成物里的**编号引用**（`refLink`）
 *
 * ## 它为什么必须在生成器里（而不是让 gen-doc-refs 事后展开）
 *
 * 生成物先于静态审计落盘（`gate.d/06-facts-write.mjs`）。若生成器产出**记号**
 * （`[[编号]]`），那一瞬间 D-007（锚点）与 RF-003（手工地址）看到的就不是最终形态——
 * 9f49d454 修的正是这个次序病。所以生成器要**直接产出展开后的链接**，让随后的一切
 * 审计面对终态。
 *
 * ## 病历（这条最值得留）
 *
 * 9f49d454 的提交消息写着「新增 `refLink(id, label)`」，diff 里却**只有用法没有定义**：
 * 六处调用写进了双引号字符串，`${refLink(...)}` 原样落进 RE-006。两类守卫同时失效——
 * 链接审计看不见它（那不是 markdown 链接），`--check` 也看不见它（生成器确定性地产出
 * 同一份坏内容，永远「一致」）。**消息声称存在而代码里不存在的函数，比没有函数更坏**：
 * 它让所有人以为那条路已经通了。执行方是 `facts.test.mjs` 的「生成物里不得留下字面
 * `refLink(`」。
 *
 * ## 为什么单独一个文件、不住 `_lib.mjs`
 *
 * `_lib.mjs` 是 runner 与端到端夹具共用的契约模块，夹具只复制它一个（facts.test.mjs
 * 的 `runIn`）——它因此必须**只依赖 node 内置模块**。本模块要读登记表
 * （`registry-lib`），住进去会让夹具在 temp 目录里找不到 `registry-lib.mjs` 而崩。
 *
 * ## 形态纪律
 *
 * 产出必须与 gen-doc-refs 展开 `[[编号|文字]]` 的结果**逐字一致**
 * （`[文字](地址 "编号")`）：RF-002 按标题里的编号重算地址，形态不一致就会被它反复
 * 「修复」，而 `--check` 又判漂移——两份机器互相打架，红的原因却不是漂移。
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { loadEntries, relFrom } from '../registry-lib.mjs'

/**
 * 造一个 `refLink(id, label?, frag?)`。
 *
 * @param root    仓根（登记表的装载点）
 * @param fromDir 产物所在目录（仓根相对）——相对地址从它算起
 * @returns (id, label = id, frag = '') => `[label](地址#锚点 "id")`；编号未登记**抛错**
 *          （生成失败好过产出一条死链：RF-001 在文档侧判同一条，这里是生成侧的同一条）
 *
 * `frag` 形如 `#23-前端--宿主路由`：与 gen-doc-refs 展开 `[[编号#锚点|文字]]` 的结果同形态。
 */
export function createRefLink(root, fromDir) {
  const { entries } = loadEntries(root)
  const byId = new Map()
  for (const e of entries) {
    if (e.meta && typeof e.meta.id === 'string' && !byId.has(e.meta.id)) byId.set(e.meta.id, e)
  }
  return function refLink(id, label = id, frag = '') {
    const target = byId.get(id)
    if (!target) {
      throw new Error(
        `refLink(\`${id}\`)：编号未登记——生成物里的站内链接必须指向已登记条目` +
          `（拼错？还是目标还没登记？）`,
      )
    }
    return `[${label}](${relFrom(fromDir, target.rel)}${frag} "${id}")`
  }
}

// 直接执行时自检：造一个、打一条，供人肉眼核对形态（不参与门禁判定）。
const MAIN =
  process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
if (MAIN) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  console.log(createRefLink(root, 'docs/ref')('RE-003-ROUTES', '003-ROUTES.md'))
}
