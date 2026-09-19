/**
 * useVdfsPrompt —— 提示态交互的单测（node 环境）
 *
 * 这个 composable 是 VdfsWorkbench 里那段内联状态机搬出来的版本。锁它的理由
 * 和搬出来的理由相同：**三种瞬态交互共占一个详情槽**，靠一个判别式互斥——
 * 一旦互斥被破坏，界面上就会同时出现两个提示（原先那种「若干布尔各自复位」
 * 的写法正是漏一处就叠一个）。
 *
 * 六个断言面：
 * 1. **互斥**：任何时刻只有一个提示态，切换不叠加；
 * 2. **选类型的两条路**：单类型直接落详情页、多类型进选择态；
 *    `source = 'file'` 的类型走「选文件」而不是详情页；
 * 3. **动作行装配**：类型清单即动作行；「上一步」只在多类型时出现；
 *    未选文件时「导入」禁用（避免点了没反应）；
 * 4. **成败语义**：写操作成功则收起，失败则**保持**（否则用户看不到错误）；
 * 5. **重命名锚在选中项上**：选中被清空时提示自动收起；
 * 6. 进入提示态即清空上一条详情级错误（提示是新的开始，不继承旧错）。
 */

import { describe, expect, it } from 'vitest'
import { effectScope, nextTick, ref, shallowRef } from 'vue'
import { useVdfsPrompt } from '../useVdfsPrompt'
import {
  VDFS_NEW_SOURCE_FILE,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'

/** 造一个新建类型（只填判定要用到的键） */
function type(ext: string, source?: string): VdfsNewType {
  return { ext, title: ext, ...(source ? { source } : {}) }
}

function node(name: string): VdfsNode {
  return {
    path: `@vfs/mcp/${name}`,
    name,
    title: name,
    kind: 'mcp',
    status: 'active',
    access: 'rw',
  } as VdfsNode
}

interface Harness {
  p: ReturnType<typeof useVdfsPrompt>
  types: ReturnType<typeof ref<VdfsNewType[]>>
  selected: ReturnType<typeof shallowRef<VdfsNode | null>>
  error: ReturnType<typeof ref<string>>
  started: VdfsNewType[]
  imported: Array<{ t: VdfsNewType; f: File }>
  renamed: string[]
  /** 让「写后端」成功或失败 */
  succeed: ReturnType<typeof ref<boolean>>
}

function setup(types: VdfsNewType[], selectedNode: VdfsNode | null = null): Harness {
  const t = ref(types)
  const selected = shallowRef<VdfsNode | null>(selectedNode)
  const error = ref('旧错误')
  const succeed = ref(true)
  const started: VdfsNewType[] = []
  const imported: Array<{ t: VdfsNewType; f: File }> = []
  const renamed: string[] = []

  // composable 内部有 watch，必须在 effect scope 里跑
  const scope = effectScope()
  const p = scope.run(() =>
    useVdfsPrompt({
      creatableTypes: t,
      cwd: ref('@vfs/mcp'),
      selectedNode: selected,
      saving: ref(false),
      error,
      startNew: (x) => void started.push(x),
      createTypedFile: async (x, f) => {
        imported.push({ t: x, f })
        return succeed.value
      },
      renameSelected: async (name) => {
        renamed.push(name)
        return succeed.value
      },
    }),
  )!

  return { p, types: t, selected, error, started, imported, renamed, succeed }
}

/** 造一个假 File（node 环境没有真正的 File 语义，只需要 name） */
function file(name: string): File {
  return { name } as File
}

describe('useVdfsPrompt — 判别式互斥', () => {
  it('初始不占用详情槽', () => {
    const { p } = setup([type('a'), type('b')])
    expect(p.promptKind.value).toBe('none')
  })

  it('type 态下开 rename：只剩 rename，不叠加', () => {
    const { p, selected } = setup([type('a'), type('b')], node('srv'))
    p.startTypedNew()
    expect(p.promptKind.value).toBe('type')
    p.startRename()
    expect(p.promptKind.value).toBe('rename')
    // 载荷一并清空，不会残留上一次的草稿名
    expect(p.promptDraft.value).toBe('srv')
    expect(selected.value).not.toBeNull()
  })

  it('进入提示态即清掉上一条详情级错误', () => {
    const { p, error } = setup([type('a'), type('b')])
    expect(error.value).toBe('旧错误')
    p.startTypedNew()
    expect(error.value).toBe('')
  })
})

describe('useVdfsPrompt — 选类型的两条路', () => {
  it('恰好一种类型：跳过选择，直接落详情页（不占提示槽）', () => {
    const { p, started } = setup([type('session')])
    p.startTypedNew()
    expect(started).toHaveLength(1)
    expect(started[0]?.ext).toBe('session')
    expect(p.promptKind.value).toBe('none')
  })

  it('多类型：进选择态，类型清单就是动作行', () => {
    const { p, started } = setup([type('model'), type('agent')])
    p.startTypedNew()
    expect(p.promptKind.value).toBe('type')
    expect(started).toHaveLength(0)
    expect(p.promptActions.value.map((a) => a.id)).toEqual(['new:model', 'new:agent', 'cancel'])
    expect(p.promptTitle.value).toBe('新建')
  })

  it('source = file 的类型走「选文件」而不是详情页', () => {
    const { p, started } = setup([type('zip', VDFS_NEW_SOURCE_FILE)])
    p.startTypedNew()
    expect(p.promptKind.value).toBe('file')
    expect(started).toHaveLength(0)
  })

  it('点类型动作 → 落到对应那一条路', () => {
    const { p, started } = setup([type('model'), type('zip', VDFS_NEW_SOURCE_FILE)])
    p.startTypedNew()
    p.onPromptAction({ id: 'new:model', label: 'model', style: 'secondary' })
    expect(started.map((t) => t.ext)).toEqual(['model'])

    p.startTypedNew()
    p.onPromptAction({ id: 'new:zip', label: 'zip', style: 'secondary' })
    expect(p.promptKind.value).toBe('file')
  })
})

describe('useVdfsPrompt — 动作行装配与禁用', () => {
  it('未选文件时「导入」禁用，选了才可用', () => {
    const { p } = setup([type('zip', VDFS_NEW_SOURCE_FILE)])
    p.startTypedNew()
    const ids = p.promptActions.value.map((a) => a.id)
    expect(p.promptDisabled.value[ids.indexOf('import')]).toBe(true)

    p.onPromptFile({ target: { files: [file('a.zip')] } } as unknown as Event)
    expect(p.promptDisabled.value[ids.indexOf('import')]).toBe(false)
  })

  it('单类型时没有「上一步」（无处可退）', () => {
    const { p } = setup([type('zip', VDFS_NEW_SOURCE_FILE)])
    p.startTypedNew()
    expect(p.promptActions.value.some((a) => a.id === 'back')).toBe(false)
  })

  it('多类型时才有「上一步」，且点了回到选择态', () => {
    const { p } = setup([type('zip', VDFS_NEW_SOURCE_FILE), type('model')])
    p.startTypedNew()
    p.onPromptAction({ id: 'new:zip', label: 'zip', style: 'secondary' })
    expect(p.promptKind.value).toBe('file')
    p.onPromptAction({ id: 'back', label: '上一步', style: 'secondary' })
    expect(p.promptKind.value).toBe('type')
  })

  it('取消：收起并把载荷清空', async () => {
    const { p } = setup([type('model'), type('agent')])
    p.startTypedNew()
    p.onPromptAction({ id: 'cancel', label: '取消', style: 'secondary' })
    expect(p.promptKind.value).toBe('none')
    await nextTick()
    expect(p.promptActions.value).toHaveLength(0)
  })
})

describe('useVdfsPrompt — 成败语义', () => {
  it('导入成功则收起', async () => {
    const { p, imported } = setup([type('zip', VDFS_NEW_SOURCE_FILE)])
    p.startTypedNew()
    p.onPromptFile({ target: { files: [file('a.zip')] } } as unknown as Event)
    // onPromptAction 对异步动作是「发起即返回」，落地在微任务里
    p.onPromptAction({ id: 'import', label: '导入', style: 'primary' })
    await nextTick()
    expect(imported).toHaveLength(1)
    expect(p.promptKind.value).toBe('none')
  })

  it('导入失败则保持提示态（否则用户看不到错误）', async () => {
    const { p, succeed } = setup([type('zip', VDFS_NEW_SOURCE_FILE)])
    succeed.value = false
    p.startTypedNew()
    p.onPromptFile({ target: { files: [file('a.zip')] } } as unknown as Event)
    p.onPromptAction({ id: 'import', label: '导入', style: 'primary' })
    await nextTick()
    expect(p.promptKind.value).toBe('file')
  })
})

describe('useVdfsPrompt — 重命名锚在选中项上', () => {
  it('进入时预填当前名', () => {
    const { p } = setup([type('a')], node('srv'))
    p.startRename()
    expect(p.promptKind.value).toBe('rename')
    expect(p.promptDraft.value).toBe('srv')
    expect(p.promptActions.value.map((a) => a.id)).toEqual(['confirm', 'cancel'])
  })

  it('成功则收起', async () => {
    const { p, renamed } = setup([type('a')], node('srv'))
    p.startRename()
    await p.submitRename()
    expect(renamed).toEqual(['srv'])
    expect(p.promptKind.value).toBe('none')
  })

  it('失败则保持提示态', async () => {
    const { p, succeed } = setup([type('a')], node('srv'))
    succeed.value = false
    p.startRename()
    await p.submitRename()
    expect(p.promptKind.value).toBe('rename')
  })

  it('选中项被清空 → 提示自动收起（没有选中项就无从改名）', async () => {
    const { p, selected } = setup([type('a')], node('srv'))
    p.startRename()
    expect(p.promptKind.value).toBe('rename')
    selected.value = null
    await nextTick()
    expect(p.promptKind.value).toBe('none')
  })

  it('选中项被清空不影响「选类型 / 选文件」（它们不锚在选中项上）', async () => {
    const { p, selected } = setup([type('model'), type('agent')])
    p.startTypedNew()
    selected.value = null
    await nextTick()
    expect(p.promptKind.value).toBe('type')
  })

  it('没有选中项时 startRename 不进提示态', () => {
    const { p } = setup([type('a')], null)
    p.startRename()
    expect(p.promptKind.value).toBe('none')
  })
})
