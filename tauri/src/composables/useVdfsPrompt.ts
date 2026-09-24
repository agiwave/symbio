/**
 * useVdfsPrompt —— 三栏工作台的「提示态」交互（新建选入口 / 导入选文件）
 *
 * ## 为什么是一个判别式状态而不是若干布尔
 *
 * 两种瞬态交互都**占用同一个详情槽**，彼此互斥。这个不变式由 `promptKind`
 * 一个判别式表达，而不是让 `startXxx` 各自去复位别人的布尔——漏一处就是两个
 * 提示叠在同一个槽里。
 *
 * ## 本层持有什么
 *
 * 只持**输入过程中的瞬态**（选了哪个入口 / 选了哪个文件），以及「这一步给哪些
 * 动作」的装配。真干活的动作（建草稿节点、写文件）全部由调用方经 `deps` 注入
 * ——本层不认识任何资源类型，也不碰 VDFS 协议。
 *
 * 两者都套 `DetailShell`（详情槽的外壳：标题行 + 动作行 + 错误行 + 内容）。
 * 提示不是例外：先前那两份手写提示外壳（各一套标题 / 动作行 / 忙态样式）
 * 是同一结构抄了两遍。
 *
 * ## 曾经还有第三种：重命名
 *
 * 随 `vdfs/move` 整条下线了（见后端 `VdfsRequest` 的「没有 `Move`」一节）。
 * 它与其他两种不同的一点是**锚在选中项上**（没有选中项就无从改名），因此带着
 * 一条 `watch(selectedNode)` 的联动；那部分一并删除。
 */

import { computed, ref } from 'vue'
import {
  newFileNameOf,
  vdfsJoin,
  type DetailAction,
  type VdfsNewEntry,
  type VdfsNewType,
} from '@/schemas/vdfs'

/** 提示态判别：`none` = 不占用详情槽 */
export type VdfsPromptKind = 'none' | 'entry' | 'file'

/** 只读盒：`useVdfs` 返回的 ref / computed 都满足它 */
interface Box<T> {
  readonly value: T
}

export interface UseVdfsPromptDeps {
  /** 当前目录的全部新建入口（由 `new_type` 推导；空 = 不可新建） */
  newEntries: Box<VdfsNewEntry[]>
  /** 当前目录地址（导入目标地址预览用） */
  cwd: Box<string>
  /** 写操作在途（锁住整行动作） */
  saving: Box<boolean>
  /** 详情级错误（进入提示态时清掉：提示是新的开始，不继承旧错） */
  error: { value: string }
  /** 进入新建详情态（选中一张草稿节点，与选中一项同一条通道） */
  startNew: (t: VdfsNewType) => void
  /** 按入口从本地文件新建（整包导入） */
  createTypedFile: (e: VdfsNewEntry, f: File) => Promise<boolean>
}

export function useVdfsPrompt(deps: UseVdfsPromptDeps) {
  const promptKind = ref<VdfsPromptKind>('none')
  /** 提示态载荷：`file` 形态下待导入的入口 */
  const promptEntry = ref<VdfsNewEntry | null>(null)
  /** 提示态载荷：`file` 形态下已选的本地文件 */
  const promptFile = ref<File | null>(null)

  /** 进入提示态（清掉上一条详情级错误：提示是新的开始，不继承旧错） */
  function openPrompt(kind: 'entry' | 'file') {
    deps.error.value = ''
    promptKind.value = kind
  }

  /** 收起提示态（载荷一并清空，避免下次打开时看到上一次的残留） */
  function closePrompt() {
    promptKind.value = 'none'
    promptEntry.value = null
    promptFile.value = null
  }

  const promptTitle = computed(() => {
    switch (promptKind.value) {
      case 'entry':
        return '新建'
      case 'file': {
        // 入口自己的展示名（主入口取类型名、导入入口取包名），故「导入技能包」成立
        return `导入${promptEntry.value?.label ?? ''}`
      }
      default:
        return ''
    }
  })

  /**
   * 提示态的行动作行 + 等长禁用标记（**同源计算**：两处各算一遍必然漂移）。
   *
   * 「选入口」的清单**就是**动作行：入口之间是并列的等价选择，「选一个 ⇒ 前进」
   * 正是动作的语义；用一行按钮表达，既不必另写一套列表渲染与样式，也不再需要
   * 把 `ext` 摆给用户看。入口动作 id 取 `new:type` / `new:import`（至多两条，
   * 见 `schemas/vdfs.vdfsNewEntries`），与语义动作 id（save / delete / …）不冲突，
   * 故一律渲染为文字按钮。
   */
  const promptBar = computed(() => {
    const actions: DetailAction[] = []
    const disabled: boolean[] = []
    const add = (a: DetailAction, off = false) => {
      actions.push(a)
      disabled.push(off)
    }
    switch (promptKind.value) {
      case 'entry':
        for (const e of deps.newEntries.value) {
          add({ id: e.id, label: e.label, style: 'secondary' })
        }
        add({ id: 'cancel', label: '取消', style: 'secondary' })
        break
      case 'file':
        // 未选文件时「导入」不可点（选文件与导入是两步，避免点了没反应）
        add({ id: 'import', label: '导入', style: 'primary' }, !promptFile.value)
        // 多于一条入口时才有「上一步」（回到入口选择）
        if (deps.newEntries.value.length > 1) {
          add({ id: 'back', label: '上一步', style: 'secondary' })
        }
        add({ id: 'cancel', label: '取消', style: 'secondary' })
        break
    }
    return { actions, disabled }
  })

  const promptActions = computed(() => promptBar.value.actions)
  const promptDisabled = computed(() => promptBar.value.disabled)
  /** 提示态的忙态：写操作（导入）在途时锁住整行 */
  const promptBusy = computed(() => promptActions.value.map(() => deps.saving.value))

  /** 文件选择器的接受类型（入口的扩展名；未声明则不限） */
  const promptAccept = computed(() =>
    promptEntry.value?.ext ? `.${promptEntry.value.ext}` : undefined,
  )
  /** 该入口的语义说明（provider 下发；没有就不显示） */
  const promptDescription = computed(() => promptEntry.value?.description ?? '')

  /** 导入地址预览（目标名由**文件名**推导，故用户不填名） */
  const typedFilePreview = computed(() => {
    const e = promptEntry.value
    if (!e) return ''
    const f = promptFile.value
    if (!f) return vdfsJoin(deps.cwd.value, `<文件名>${e.ext ? `.${e.ext}` : ''}`)
    return vdfsJoin(deps.cwd.value, newFileNameOf(f.name, e.ext))
  })

  function onPromptAction(a: DetailAction) {
    // 选入口：`new:` 前缀 → 落到「进详情页」或「选文件」两条路之一
    if (promptKind.value === 'entry' && a.id.startsWith('new:')) {
      const e = deps.newEntries.value.find((x) => x.id === a.id)
      if (e) chooseEntry(e)
      return
    }
    switch (a.id) {
      case 'import':
        void submitTypedFile()
        return
      case 'back':
        // 回到入口选择（只有多入口时动作行才给出这一项）
        promptKind.value = 'entry'
        return
      case 'cancel':
        closePrompt()
    }
  }

  function startNewEntry() {
    // 先整体收起当前提示（载荷一并清空）——「新建」可能是从另一个提示态过来的，
    // 判别式只保证**界面**上不叠两个提示，残留载荷得靠这一步清掉。
    closePrompt()
    const entries = deps.newEntries.value
    // 没有入口（该目录由系统管理）：界面本就不给「新建」按钮，这里兜底不动作——
    // 进了「选入口」态就会渲染出一行只有「取消」的动作行。
    const only = entries[0]
    if (!only) return
    // 恰好一条入口：跳过入口选择，直接落到那一条路
    if (entries.length === 1) {
      chooseEntry(only)
      return
    }
    openPrompt('entry')
  }

  /** 选定入口 → 落到「进详情页」或「选文件」两条路之一 */
  function chooseEntry(e: VdfsNewEntry) {
    if (e.fromFile) {
      promptEntry.value = e
      promptFile.value = null
      openPrompt('file')
      return
    }
    // 进详情页 = 收起提示 + 选中一张草稿节点（与选中一项同一条通道）
    closePrompt()
    deps.startNew(e.type)
  }

  function onPromptFile(e: Event) {
    const input = e.target as HTMLInputElement
    promptFile.value = input.files?.[0] ?? null
    deps.error.value = ''
  }

  async function submitTypedFile() {
    const e = promptEntry.value
    const f = promptFile.value
    if (!e || !f) return
    if (await deps.createTypedFile(e, f)) closePrompt()
  }

  return {
    promptKind,
    promptEntry,
    promptFile,
    promptTitle,
    promptActions,
    promptDisabled,
    promptBusy,
    promptAccept,
    promptDescription,
    typedFilePreview,
    onPromptAction,
    onPromptFile,
    startNewEntry,
  }
}
