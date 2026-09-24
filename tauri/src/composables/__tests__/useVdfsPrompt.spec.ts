/**
 * useVdfsPrompt —— 提示态交互的单测（node 环境）
 *
 * 这个 composable 是 VdfsWorkbench 里那段内联状态机搬出来的版本。锁它的理由
 * 和搬出来的理由相同：**两种瞬态交互共占一个详情槽**，靠一个判别式互斥——
 * 一旦互斥被破坏，界面上就会同时出现两个提示（原先那种「若干布尔各自复位」
 * 的写法正是漏一处就叠一个）。
 *
 * ## 类型与入口是两件事
 *
 * 一个目录只声明**一类**东西（`new_type`，至多一个），但这一类东西可以有**两条
 * 入口**（主入口 + 整包导入，由 `VdfsNewType.import` 表达）。所以：
 *
 * - 一条入口 ⇒ 跳过选择，直接落到那条路；
 * - 两条入口 ⇒ 先进「选入口」态（清单就是动作行）；
 * - 没有类型 ⇒ 没有入口（添加按钮本就不可见，这里兜底不动作）。
 *
 * 五个断言面：
 * 1. **互斥**：任何时刻只有一个提示态，切换不叠加；
 * 2. **入口推导与两条路**：`vdfsNewEntries` 的产物即入口；主入口按 `source`
 *    分流（详情页 / 选文件），导入入口恒走选文件；
 * 3. **动作行装配**：入口清单即动作行；「上一步」只在多入口时出现；
 *    未选文件时「导入」禁用（避免点了没反应）；
 * 4. **成败语义**：写操作成功则收起，失败则**保持**（否则用户看不到错误）；
 * 5. 进入提示态即清空上一条详情级错误（提示是新的开始，不继承旧错）。
 *
 * （曾经还有第三种提示——重命名。它随 `vdfs/move` 整条下线了，连同
 * 「锚在选中项上」的那条 `watch`。）
 */

import { describe, expect, it } from 'vitest'
import { computed, effectScope, nextTick, ref } from 'vue'
import { useVdfsPrompt } from '../useVdfsPrompt'
import {
  VDFS_NEW_ENTRY_IMPORT,
  VDFS_NEW_ENTRY_TYPE,
  VDFS_NEW_SOURCE_FILE,
  vdfsNewEntries,
  type VdfsNewEntry,
  type VdfsNewType,
} from '@/schemas/vdfs'

/** 造一个新建类型（只填判定要用到的键） */
function type(ext: string, extra: Partial<VdfsNewType> = {}): VdfsNewType {
  return { ext, title: ext, ...extra }
}

/** 造一个「表单新建 + 整包导入」的类型（skill / mcp 的形状） */
function typeWithImport(ext: string, packExt = 'zip'): VdfsNewType {
  return type(ext, { import: { ext: packExt, title: `${ext}包` } })
}

interface Harness {
  p: ReturnType<typeof useVdfsPrompt>
  error: ReturnType<typeof ref<string>>
  started: VdfsNewType[]
  imported: Array<{ e: VdfsNewEntry; f: File }>
  /** 让「写后端」成功或失败 */
  succeed: ReturnType<typeof ref<boolean>>
}

function setup(newType: VdfsNewType | null): Harness {
  const error = ref('旧错误')
  const succeed = ref(true)
  const started: VdfsNewType[] = []
  const imported: Array<{ e: VdfsNewEntry; f: File }> = []

  // composable 内部用 computed 派生动作行，必须在 effect scope 里跑
  const scope = effectScope()
  const p = scope.run(() =>
    useVdfsPrompt({
      newEntries: computed(() => vdfsNewEntries(newType)),
      cwd: ref('@vfs/mcp'),
      saving: ref(false),
      error,
      startNew: (x) => void started.push(x),
      createTypedFile: async (e, f) => {
        imported.push({ e, f })
        return succeed.value
      },
    }),
  )!

  return { p, error, started, imported, succeed }
}

/** 造一个假 File（node 环境没有真正的 File 语义，只需要 name） */
function file(name: string): File {
  return { name } as File
}

/** 触发一次「点新建」 */
function clickNew(h: Harness) {
  h.p.startNewEntry()
}

describe('useVdfsPrompt — 判别式互斥', () => {
  it('初始不占用详情槽', () => {
    const h = setup(typeWithImport('skill'))
    expect(h.p.promptKind.value).toBe('none')
  })

  it('file 态下再点「新建」⇒ 回到入口态，不叠加两个提示', () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_IMPORT, label: 'skill包', style: 'secondary' })
    expect(h.p.promptKind.value).toBe('file')

    clickNew(h)
    expect(h.p.promptKind.value).toBe('entry')
    // 载荷一并清空：上一次选的文件不会跟到这一次
    expect(h.p.promptFile.value).toBeNull()
  })

  it('进入提示态即清掉上一条详情级错误', () => {
    const h = setup(typeWithImport('skill'))
    expect(h.error.value).toBe('旧错误')
    clickNew(h)
    expect(h.error.value).toBe('')
  })
})

describe('useVdfsPrompt — 入口推导与两条路', () => {
  it('无类型：没有入口，点新建不动作', () => {
    const h = setup(null)
    clickNew(h)
    expect(h.p.promptKind.value).toBe('none')
    expect(h.p.promptActions.value).toHaveLength(0)
    expect(h.started).toHaveLength(0)
  })

  it('单入口（表单新建）：跳过选择，直接落详情页（不占提示槽）', () => {
    const h = setup(type('session'))
    clickNew(h)
    expect(h.started.map((t) => t.ext)).toEqual(['session'])
    expect(h.p.promptKind.value).toBe('none')
  })

  it('单入口（主入口即选文件）：直接进「选文件」态', () => {
    const h = setup(type('zip', { source: VDFS_NEW_SOURCE_FILE }))
    clickNew(h)
    expect(h.p.promptKind.value).toBe('file')
    expect(h.started).toHaveLength(0)
  })

  it('两入口（表单 + 整包导入）：先进选入口态，清单就是动作行', () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    expect(h.p.promptKind.value).toBe('entry')
    expect(h.started).toHaveLength(0)
    expect(h.p.promptActions.value.map((a) => a.id)).toEqual([
      VDFS_NEW_ENTRY_TYPE,
      VDFS_NEW_ENTRY_IMPORT,
      'cancel',
    ])
    // 展示名取入口自己的（主入口 = 类型名，导入入口 = 包名）
    expect(h.p.promptActions.value.map((a) => a.label)).toEqual(['skill', 'skill包', '取消'])
    expect(h.p.promptTitle.value).toBe('新建')
  })

  it('点主入口 → 落详情页；点导入入口 → 进「选文件」态', () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_TYPE, label: 'skill', style: 'secondary' })
    expect(h.started.map((t) => t.ext)).toEqual(['skill'])

    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_IMPORT, label: 'skill包', style: 'secondary' })
    expect(h.p.promptKind.value).toBe('file')
  })

  it('导入入口的扩展名取包而不是类型（文件选择器与目标名都用它）', () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_IMPORT, label: 'skill包', style: 'secondary' })
    expect(h.p.promptAccept.value).toBe('.zip')
    expect(h.p.typedFilePreview.value).toBe('@vfs/mcp/<文件名>.zip')

    h.p.onPromptFile({ target: { files: [file('demo.tar.gz')] } } as unknown as Event)
    expect(h.p.typedFilePreview.value).toBe('@vfs/mcp/demo.tar.zip')
  })

  it('主入口的扩展名取类型自己的', () => {
    const h = setup(type('zip', { source: VDFS_NEW_SOURCE_FILE }))
    clickNew(h)
    expect(h.p.promptAccept.value).toBe('.zip')
    h.p.onPromptFile({ target: { files: [file('demo.zip')] } } as unknown as Event)
    expect(h.p.typedFilePreview.value).toBe('@vfs/mcp/demo.zip')
  })
})

describe('useVdfsPrompt — 动作行装配与禁用', () => {
  it('未选文件时「导入」禁用，选了才可用', () => {
    const h = setup(type('zip', { source: VDFS_NEW_SOURCE_FILE }))
    clickNew(h)
    const ids = h.p.promptActions.value.map((a) => a.id)
    expect(h.p.promptDisabled.value[ids.indexOf('import')]).toBe(true)

    h.p.onPromptFile({ target: { files: [file('a.zip')] } } as unknown as Event)
    expect(h.p.promptDisabled.value[ids.indexOf('import')]).toBe(false)
  })

  it('单入口时没有「上一步」（无处可退）', () => {
    const h = setup(type('zip', { source: VDFS_NEW_SOURCE_FILE }))
    clickNew(h)
    expect(h.p.promptActions.value.some((a) => a.id === 'back')).toBe(false)
  })

  it('多入口时才有「上一步」，且点了回到选入口态', () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_IMPORT, label: 'skill包', style: 'secondary' })
    expect(h.p.promptKind.value).toBe('file')
    h.p.onPromptAction({ id: 'back', label: '上一步', style: 'secondary' })
    expect(h.p.promptKind.value).toBe('entry')
  })

  it('取消：收起并把载荷清空', async () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    h.p.onPromptAction({ id: 'cancel', label: '取消', style: 'secondary' })
    expect(h.p.promptKind.value).toBe('none')
    await nextTick()
    expect(h.p.promptActions.value).toHaveLength(0)
  })
})

describe('useVdfsPrompt — 成败语义', () => {
  it('导入成功则收起（并把**入口**交给写动作，扩展名由它决定）', async () => {
    const h = setup(typeWithImport('skill'))
    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_IMPORT, label: 'skill包', style: 'secondary' })
    h.p.onPromptFile({ target: { files: [file('a.zip')] } } as unknown as Event)
    // onPromptAction 对异步动作是「发起即返回」，落地在微任务里
    h.p.onPromptAction({ id: 'import', label: '导入', style: 'primary' })
    await nextTick()
    expect(h.imported).toHaveLength(1)
    expect(h.imported[0]?.e.ext).toBe('zip')
    expect(h.imported[0]?.e.type.ext).toBe('skill')
    expect(h.p.promptKind.value).toBe('none')
  })

  it('导入失败则保持提示态（否则用户看不到错误）', async () => {
    const h = setup(typeWithImport('skill'))
    h.succeed.value = false
    clickNew(h)
    h.p.onPromptAction({ id: VDFS_NEW_ENTRY_IMPORT, label: 'skill包', style: 'secondary' })
    h.p.onPromptFile({ target: { files: [file('a.zip')] } } as unknown as Event)
    h.p.onPromptAction({ id: 'import', label: '导入', style: 'primary' })
    await nextTick()
    expect(h.p.promptKind.value).toBe('file')
  })
})
