/**
 * useVdfsPrompt —— 三栏工作台的「提示态」交互（新建选类型 / 导入选文件 / 重命名）
 *
 * ## 为什么是一个判别式状态而不是若干布尔
 *
 * 三种瞬态交互都**占用同一个详情槽**，彼此互斥。这个不变式由 `promptKind`
 * 一个判别式表达，而不是让 `startXxx` 各自去复位别人的布尔——漏一处就是两个
 * 提示叠在同一个槽里。
 *
 * ## 本层持有什么
 *
 * 只持**输入过程中的瞬态**（选了哪种类型 / 选了哪个文件 / 改名草稿），以及
 * 「这一步给哪些动作」的装配。真干活的动作（建草稿节点、写文件、改名字）
 * 全部由调用方经 `deps` 注入——本层不认识任何资源类型，也不碰 VDFS 协议。
 *
 * 三者都套 `DetailShell`（详情槽的外壳：标题行 + 动作行 + 错误行 + 内容）。
 * 提示不是例外：先前那两份手写提示外壳（各一套标题 / 动作行 / 忙态样式）
 * 是同一结构抄了两遍。
 */

import { computed, ref, watch } from 'vue'
import {
  VDFS_NEW_SOURCE_FILE,
  newFileNameOf,
  vdfsJoin,
  vdfsParent,
  type DetailAction,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'

/** 提示态判别：`none` = 不占用详情槽 */
export type VdfsPromptKind = 'none' | 'type' | 'file' | 'rename'

/** 只读盒：`useVdfs` 返回的 ref / computed 都满足它 */
interface Box<T> {
  readonly value: T
}

export interface UseVdfsPromptDeps {
  /** 当前目录可接受的新建类型（目录节点 `new_types` 声明） */
  creatableTypes: Box<VdfsNewType[]>
  /** 当前目录地址（导入 / 改名的目标地址预览用） */
  cwd: Box<string>
  /** 选中节点（重命名锚在它身上；没有选中项就无从改名） */
  selectedNode: Box<VdfsNode | null>
  /** 写操作在途（锁住整行动作） */
  saving: Box<boolean>
  /** 详情级错误（进入提示态时清掉：提示是新的开始，不继承旧错） */
  error: { value: string }
  /** 进入新建详情态（选中一张草稿节点，与选中一项同一条通道） */
  startNew: (t: VdfsNewType) => void
  /** 按类型从本地文件新建（整包导入） */
  createTypedFile: (t: VdfsNewType, f: File) => Promise<boolean>
  /** 重命名选中节点 */
  renameSelected: (name: string) => Promise<boolean>
}

export function useVdfsPrompt(deps: UseVdfsPromptDeps) {
  const promptKind = ref<VdfsPromptKind>('none')
  /** 提示态载荷：`file` 形态下待导入的类型 */
  const promptType = ref<VdfsNewType | null>(null)
  /** 提示态载荷：`file` 形态下已选的本地文件 */
  const promptFile = ref<File | null>(null)
  /** 提示态载荷：重命名的草稿名 */
  const promptDraft = ref('')

  /** 进入提示态（清掉上一条详情级错误：提示是新的开始，不继承旧错） */
  function openPrompt(kind: 'type' | 'file' | 'rename') {
    deps.error.value = ''
    promptKind.value = kind
  }

  /** 收起提示态（载荷一并清空，避免下次打开时看到上一次的残留） */
  function closePrompt() {
    promptKind.value = 'none'
    promptType.value = null
    promptFile.value = null
    promptDraft.value = ''
  }

  const promptTitle = computed(() => {
    switch (promptKind.value) {
      case 'type':
        return '新建'
      case 'file': {
        const t = promptType.value
        return `导入${t?.title || t?.ext || ''}`
      }
      case 'rename':
        return '重命名'
      default:
        return ''
    }
  })

  /**
   * 提示态的行动作行 + 等长禁用标记（**同源计算**：两处各算一遍必然漂移）。
   *
   * 「选类型」的清单**就是**动作行：类型之间是并列的等价选择，「选一个 ⇒ 前进」
   * 正是动作的语义；用一行按钮表达，既不必另写一套列表渲染与样式，也不再需要
   * 把 `ext` 摆给用户看。类型动作 id 取 `new:<ext>`，与语义动作 id
   * （save / delete / …）不冲突，故一律渲染为文字按钮。
   */
  const promptBar = computed(() => {
    const actions: DetailAction[] = []
    const disabled: boolean[] = []
    const add = (a: DetailAction, off = false) => {
      actions.push(a)
      disabled.push(off)
    }
    switch (promptKind.value) {
      case 'type':
        for (const t of deps.creatableTypes.value) {
          add({ id: `new:${t.ext}`, label: t.title || t.ext, style: 'secondary' })
        }
        add({ id: 'cancel', label: '取消', style: 'secondary' })
        break
      case 'file':
        // 未选文件时「导入」不可点（选文件与导入是两步，避免点了没反应）
        add({ id: 'import', label: '导入', style: 'primary' }, !promptFile.value)
        // 多于一种类型时才有「上一步」（回到类型选择）
        if (deps.creatableTypes.value.length > 1) {
          add({ id: 'back', label: '上一步', style: 'secondary' })
        }
        add({ id: 'cancel', label: '取消', style: 'secondary' })
        break
      case 'rename':
        add({ id: 'confirm', label: '确定', style: 'primary' })
        add({ id: 'cancel', label: '取消', style: 'secondary' })
        break
    }
    return { actions, disabled }
  })

  const promptActions = computed(() => promptBar.value.actions)
  const promptDisabled = computed(() => promptBar.value.disabled)
  /** 提示态的忙态：写操作（导入 / 重命名）在途时锁住整行 */
  const promptBusy = computed(() => promptActions.value.map(() => deps.saving.value))

  /** 文件选择器的接受类型（类型声明的呈现扩展名；未声明则不限） */
  const promptAccept = computed(() =>
    promptType.value?.ext ? `.${promptType.value.ext}` : undefined,
  )
  /** 该类型的语义说明（provider 下发；没有就不显示） */
  const promptDescription = computed(() => promptType.value?.description ?? '')

  /** 导入地址预览（目标名由**文件名**推导，故用户不填名） */
  const typedFilePreview = computed(() => {
    const t = promptType.value
    if (!t) return ''
    const f = promptFile.value
    if (!f) return vdfsJoin(deps.cwd.value, `<文件名>${t.ext ? `.${t.ext}` : ''}`)
    return vdfsJoin(deps.cwd.value, newFileNameOf(f.name, t.ext))
  })

  const renamePreview = computed(() => {
    const node = deps.selectedNode.value
    return node ? vdfsJoin(vdfsParent(node.path), promptDraft.value || '<名称>') : ''
  })

  function onPromptAction(a: DetailAction) {
    // 选类型：`new:<ext>` → 落到「进详情页」或「选文件」两条路之一
    if (promptKind.value === 'type' && a.id.startsWith('new:')) {
      const t = deps.creatableTypes.value.find((x) => x.ext === a.id.slice(4))
      if (t) chooseType(t)
      return
    }
    switch (a.id) {
      case 'import':
        void submitTypedFile()
        return
      case 'back':
        // 回到类型选择（只有多类型时动作行才给出这一项）
        promptKind.value = 'type'
        return
      case 'confirm':
        void submitRename()
        return
      case 'cancel':
        closePrompt()
    }
  }

  function startTypedNew() {
    // 先整体收起当前提示（载荷一并清空）——「新建」可能是在重命名提示开着时点的，
    // 判别式只保证**界面**上不叠两个提示，残留的 `promptDraft` 得靠这一步清掉。
    closePrompt()
    const types = deps.creatableTypes.value
    // 恰好一种类型：跳过类型选择，直接落到那一条路
    const only = types[0]
    if (types.length === 1 && only) {
      chooseType(only)
      return
    }
    openPrompt('type')
  }

  /** 选定类型 → 落到「进详情页」或「选文件」两条路之一 */
  function chooseType(t: VdfsNewType) {
    if (t.source === VDFS_NEW_SOURCE_FILE) {
      promptType.value = t
      promptFile.value = null
      openPrompt('file')
      return
    }
    // 进详情页 = 收起提示 + 选中一张草稿节点（与选中一项同一条通道）
    closePrompt()
    deps.startNew(t)
  }

  function onPromptFile(e: Event) {
    const input = e.target as HTMLInputElement
    promptFile.value = input.files?.[0] ?? null
    deps.error.value = ''
  }

  async function submitTypedFile() {
    const t = promptType.value
    const f = promptFile.value
    if (!t || !f) return
    if (await deps.createTypedFile(t, f)) closePrompt()
  }

  function startRename() {
    const node = deps.selectedNode.value
    if (!node) return
    promptDraft.value = node.name
    openPrompt('rename')
  }

  async function submitRename() {
    if (promptKind.value !== 'rename') return
    if (await deps.renameSelected(promptDraft.value)) closePrompt()
  }

  // 选中被清掉（刷新收敛判定该项已消失 / 用户点了别处）时收起重命名提示：
  // 它锚在选中项上——没有选中项就无从改名。
  // 「选类型 / 选文件」不锚在选中项上，故不受影响。
  watch(
    () => deps.selectedNode.value,
    (n) => {
      if (!n && promptKind.value === 'rename') closePrompt()
    },
  )

  return {
    promptKind,
    promptType,
    promptFile,
    promptDraft,
    promptTitle,
    promptActions,
    promptDisabled,
    promptBusy,
    promptAccept,
    promptDescription,
    typedFilePreview,
    renamePreview,
    onPromptAction,
    onPromptFile,
    startTypedNew,
    startRename,
    submitRename,
  }
}
