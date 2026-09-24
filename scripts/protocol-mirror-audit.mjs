#!/usr/bin/env node
/**
 * protocol-mirror-audit — 跨栈协议常量的**镜像一致性**守卫
 *
 * ## 它守的是什么（五组）
 *
 * **A. 常量镜像**（后端与前端必须**逐字相等**）
 *   前端持有的 `VDFS_*` 常量是后端协议词的**副本**——它拿这些词拼地址、认目录、
 *   选渲染器、判状态。副本漂移不会有任何测试变红，只会在运行期表现为
 *   「消息读不到 / 渲染器选错 / 状态判反」，所以要有专门的检查。
 *
 *   本组**自动发现**：扫后端两个常量源 + 前端 `schemas/vdfs.ts`，取同名交集逐条
 *   比对。新增一个常量即自动进入守卫——**不需要改本脚本**。此前是手工登记 3 条，
 *   剩下 26 条无人看守（`docs/archive/architecture-health-check-2026-09.md` F-5）。
 *   名字不同的镜像登记在 `ALIASES`；前端自持（后端无对应）的常量必须登记在
 *   `LOCAL_ONLY` 并写明理由——**"没登记"会报错**，所以不会有静默的漏网。
 *
 * **B. 缺席检查**（前端**不得再出现**）
 *   会话的两个地址段（挂载段 `session`、转写段 `消息`）已改为**运行期发现**
 *   （见 `tauri/src/services/vdfsScheme.ts`），前端不再持有它们。这两条保证
 *   它们不会悄悄回来——回来就是第二份真相。
 *
 * **C. 闭集词表**（取值**集合**必须相等）
 *   后端 `#[serde(rename_all = "snake_case")]` 的**闭集枚举**是跨栈契约：前端按
 *   这些词分发渲染与业务规则。枚举加一个变体而前端词表没跟，那条变更会被前端
 *   当成「未知词」静默丢弃——`aborted` 曾因此让中止后的 Turn 显示成已完成、
 *   重试入口不出现。
 *
 *   比对的是**取值集合**，不是顺序（顺序对前端无意义，故不比）。
 *   只查 `rename_all = "snake_case"` 的枚举：少了这个声明，serde 会用变体名原名
 *   （`ToolCall`），下面那套转换就会「看起来正确」而实际全错——所以声明本身也是
 *   被检查项。
 *
 * **D. 结构体字段**（前端字段必须能在后端找到）
 *   A / C 两组守的是**取值**，这一组守**字段名**。后端把 `parent_id` 改名、前端
 *   还读旧名时，读到的永远是 `undefined`——**编译期不报、运行期不报**，只表现为
 *   "某个功能悄悄不工作了"。它比常量漂移更难发现，所以要有守卫。
 *
 *   只查一个方向：**前端持有的字段必须在后端线格式里存在**（或登记在 `tsLocal`
 *   作为"前端自持"）。反方向不查——前端不必镜像后端全部字段，多一个字段不构成
 *   问题，少一个才是。只比字段名不比类型：类型映射正则读不出来，而"改字段名"本就是
 *   后端最常见的契约变更。
 *
 * **E. 跨栈导航头**（`Corresponding Frontend` 必须指向真实文件）
 *   后端协议文件顶部写 `// Corresponding Frontend: <路径>`，指向它的前端镜像——
 *   这是**人**在两边之间跳转的入口。它腐烂得很安静：前端改名 / 删除后，头照旧
 *   指向旧路径，没人会被告知。实测 8 条里 **7 条悬空**，其中 6 条指向
 *   `tauri/src/protocols/`，而那个目录**从未在版本史里出现过**。
 *
 *   约定因此收紧为：**写了就必须指向真实文件；前端没有镜像就别写**——假指针比
 *   没有更糟（不写只是缺个跳转，写假的会让人以为那边有人在看）。
 *
 * ## 它**不**声称什么
 *
 * 判定基于正则读源码，不是 AST：注释掉的常量同样会命中（缺席检查因此偏严，
 * 这符合它的意图——连注释里都不该教人写回去）。
 *
 * D 组不解析 `Option<T>` / 泛型 / 嵌套类型，也不处理结构体级 `rename_all`
 * （遇到会**报错**而不是默默算错）。
 *
 * ## 用法
 *
 *   node scripts/protocol-mirror-audit.mjs                # 审计本仓库
 *   node scripts/protocol-mirror-audit.mjs --repo=<dir>   # 换仓库根（回归测试用）
 *
 * 退出码：0 = 全部通过；1 = 存在不一致 / 缺失 / 不应出现的常量。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

// 配色统一走 `color.mjs`。**不要**在这里自己写 `\x1b`：本脚本此前手写了一份，
// 且把 `\x1b` 写丢了（`[31m` 少了 ESC）⇒ 终端与 CI 日志里显示的是**字面量**
// `[31m` 而不是红色，而两个回归测试都设了 `NO_COLOR=1`，正好绕过这条分支，
// 于是「守卫的输出坏了但守卫仍绿」。收成一份实现是唯一能根治这件事的办法。
import { red, green, yellow, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRepo = path.resolve(scriptDir, '..')

function argValue(name) {
  const hit = process.argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : null
}

const REPO = argValue('repo') ? path.resolve(argValue('repo')) : defaultRepo

const OK = '✓'
const BAD = '✗'

// ==================== A. 常量镜像 ====================

const VDFS_PROVIDER_RS = 'symbio/src/symbio_core/vdfs_provider.rs'
const VDFS_PROTOCOL_RS = 'symbio/src/plugins/vdfs/protocol.rs'
const VDFS_TS = 'tauri/src/schemas/vdfs.ts'

const CHAT_MESSAGE_RS = 'symbio/src/symbio_core/schemas/session/chat_message.rs'
const CHAT_MESSAGE_TS = 'tauri/src/schemas/chat_message.ts'
const DETAIL_RS = 'symbio/src/symbio_core/schemas/detail.rs'
const FORM_TS = 'tauri/src/schemas/vdfs-form.ts'
const RISK_LEVEL_RS = 'symbio/src/plugins/local/policy/policy_types.rs'
const SESSION_META_TS = 'tauri/src/schemas/session_meta.ts'

/** 后端常量源：自动发现其中 `VDFS_*` 前缀的 `&str` 常量 */
const RUST_CONST_FILES = [VDFS_PROVIDER_RS, VDFS_PROTOCOL_RS]

/** 前端常量源 */
const TS_CONST_FILES = [VDFS_TS]

/**
 * 名字不同、但确实是同一份协议词的镜像对。
 *
 * 为什么会有别名：前端有时要在名字里区分**用途**——`VDFS_ROOT_OP` 是"取根地址的
 * 那个 op"，名字里带 `OP` 才不会与 `schemas/vdfsRoot` 的根**地址**混淆；而后端
 * 只有一份常量名。名字不同 ⇒ 自动发现看不见它们，因此必须显式登记。
 */
const ALIASES = [
  {
    ts: 'VDFS_ROOT_OP',
    rust: { file: 'symbio/src/plugins/vdfs/protocol.rs', name: 'VDFS_ROOT' },
    what: '进入地址空间：列出虚拟根（不给地址）',
  },
  {
    ts: 'VDFS_EVENT_KIND',
    rust: { file: 'symbio/src/symbio_core/event_bus.rs', name: 'KIND_VDFS' },
    what: 'VDFS 变更的总线频道名',
  },
  {
    ts: 'VDFS_BUS_RESYNC',
    rust: { file: 'symbio/src/symbio_core/event_bus.rs', name: 'RESYNC_MARKER_TYPE' },
    what: '总线背压指令：通道曾满，消费端请重读作用域（不是一条变更）',
  },
]

/**
 * 前端**自持**的 `VDFS_*` 常量（后端没有对应协议词）。
 *
 * 这是逃生舱：能进这里说明它**不是**跨栈契约，而是前端自己的概念。
 * 每条必须写明理由——「后端没有对应」这件事本身要经得起复核。
 */
const LOCAL_ONLY = [
  {
    name: 'VDFS_NEW_ENTRY_TYPE',
    what: '「新建」提示态里**主入口**的动作 id',
    reason:
      '提示态动作行的标识，不是协议词：后端只声明「这一种类型 + 可选导入入口」，' +
      '把入口渲染成哪一行按钮、按什么 id 派发，是前端交互自己的事。' +
      '后端无从解释 `new:type` 这个字符串。',
  },
  {
    name: 'VDFS_NEW_ENTRY_IMPORT',
    what: '「新建」提示态里**整包导入入口**的动作 id',
    reason: '同上：与 `VDFS_NEW_ENTRY_TYPE` 同族的前端交互标识，非协议词。',
  },
]

// ==================== B. 缺席检查 ====================

/**
 * 前端**不得再出现**的常量。
 *
 * 会话地址段已改为运行期发现（`services/vdfsScheme.ts`）：挂载段按「挂载点声明
 * 可新建 `ext = session`」认出来，转写段按 `kind = VDFS_KIND_MESSAGES` 认出来。
 * 前端持有它们就是第二份真相——这两条防止它们悄悄回来。
 */
const ABSENT = [
  {
    name: 'VDFS_SESSION_DIR',
    what: '会话挂载段常量（应由 vdfsScheme 运行期发现）',
    pattern: /\bVDFS_SESSION_DIR\b/,
  },
  {
    name: 'VDFS_SEG_MESSAGES',
    what: '转写段常量（应由 vdfsScheme 按 kind 发现）',
    pattern: /\bVDFS_SEG_MESSAGES\b/,
  },
]

// ==================== C. 闭集词表 ====================

/**
 * 后端**闭集的取值集合** ↔ 前端**词表数组**。
 *
 * 对应关系无法自动推断（`MessageRole` ↔ `CHAT_ROLES` 名字不同），故显式登记；
 * 但登记后**取值**是自动提取比对的——枚举加变体、词表加取值，两边立刻对上账。
 *
 * 后端表达闭集有**两种**手段，两种都收：
 * - `enum` + `#[serde(rename_all = "…")]` —— 按声明的取值转换（见 `CASE_CONVERTERS`）；
 * - `constPrefix`（一组 `pub const PREFIX_*: &str = "…"`）—— 字面即线上取值。
 *
 * 只认第一种会让第二种长期无人看守：当初 `OPTION_PICK_*` 就是这么漏的——它被
 * 登记成「消费方在前端」，而前端那份其实是**独立硬编码**的第二份抄本，与 Rust
 * 常量没有任何引用关系（那套机制已于 2026-09-23 整体下线，闭集迁到
 * `detail.rs::DETAIL_PICK_*` ↔ `vdfs-form.ts::DETAIL_PICKS`；账目见
 * `docs/archive/session-options-unification.md`）。
 */
const ENUM_SETS = [
  {
    what: '消息角色',
    rust: { file: CHAT_MESSAGE_RS, enum: 'MessageRole' },
    ts: { file: CHAT_MESSAGE_TS, array: 'CHAT_ROLES' },
  },
  {
    what: '消息类型',
    rust: { file: CHAT_MESSAGE_RS, enum: 'MessageType' },
    ts: { file: CHAT_MESSAGE_TS, array: 'MESSAGE_TYPES' },
  },
  {
    what: '消息状态',
    rust: { file: CHAT_MESSAGE_RS, enum: 'MessageStatus' },
    ts: { file: CHAT_MESSAGE_TS, array: 'MESSAGE_STATUSES' },
  },
  {
    what: '会话恢复动作',
    rust: { file: CHAT_MESSAGE_RS, enum: 'ResumeAction' },
    ts: { file: CHAT_MESSAGE_TS, array: 'RESUME_ACTIONS' },
  },
  {
    what: '工具风险等级',
    // ⚠️ 这条是 `rename_all = "lowercase"`（不是 `snake_case`）：本组**自动读取**
    // 声明的取值并选用对应的转换规则，故两者都能守。此前只认 `snake_case`，
    // 于是它在前端 `schemas/session_meta.ts` 的镜像（`SESSION_RISK_LEVELS`）
    // 长期无人看守——「枚举类型对了但属性取值没覆盖到」是守卫自己的漏。
    rust: { file: RISK_LEVEL_RS, enum: 'RiskLevel' },
    ts: { file: SESSION_META_TS, array: 'SESSION_RISK_LEVELS' },
  },
  {
    what: '机制原生取值原语（详情表单）',
    // S1 把「原生取值」从级联选项体系搬进了**通用方言**：`pick` 现在是任何详情
    // 表单字段都能声明的能力（`DetailField::pick`），不再是选项专属。取值集合的
    // 唯一定义处随之迁到 `schemas/vdfs-form.ts::DETAIL_PICKS`，本条按 `DETAIL_PICK_`
    // 前缀提取后端取值逐词比对。
    rust: { file: DETAIL_RS, constPrefix: 'DETAIL_PICK_' },
    ts: { file: FORM_TS, array: 'DETAIL_PICKS' },
  },
]

// ==================== D. 结构体字段 ====================

/**
 * 后端**结构体** ↔ 前端**接口**。
 *
 * 登记原则：**前端确实镜像了它、且漂移代价高**的才进来。前端不必镜像全部结构体
 * （很多响应只是取几个字段就用掉了），把没有镜像关系的对塞进来只会制造噪音。
 *
 * 判据是「前端**逐字段**镜像了它」：字段一个不落地抄了一遍，就说明前端把这套形状
 * 当成了自己的契约——此时后端改一个字段名，前端读到的就是 `undefined`。反过来，
 * 前端只挑几个字段用的响应结构不进这里：它本来就该按需取，多抄反而不必。
 *
 * `tsLocal` 是前端**自持**的字段（后端不下发、前端自己组装的）——每条必须写明
 * 理由。没登记又对不上的，一律报错：报错信息里给出三条出路（改名 / 登记 / 删掉）。
 *
 * ## 新增一条登记的步骤（顺序有语义）
 *
 * 1. **先探，再登记**。写个一次性探针复用本文件的 `rustStructFields` /
 *    `tsInterfaceFields` 逐对比对，确认**差异为 0** 再登记。差异非 0 说明前端已经
 *    漂移了——那要**先修前端**（或确认这是有意的自持，走 `tsLocal`），而不是把
 *    红的登记进来。
 * 2. **登记**。加进 `STRUCT_SETS`，`what` 写人话（报错信息里只出现 `what` 与结构体名，
 *    不写清用途等于让人回头读源码）。
 * 3. **同步铺夹具**（`protocol-mirror-audit.test.mjs` 的 `BASE`）。**少铺一对，基线
 *    就会因「结构体 / 接口不存在」变红**——这是刻意的耦合，保证夹具与本清单不脱节。
 * 4. **跑门禁**：`node scripts/gate.mjs --only=docs,facts`（它会先跑本脚本自己的回归
 *    测试，再跑真仓库）。只跑真仓库不够——真仓库全绿只证明"现在没漂"，不证明"漂了
 *    会红"。
 */
const STRUCT_SETS = [
  {
    what: '会话消息',
    rust: { file: CHAT_MESSAGE_RS, struct: 'ChatMessage' },
    ts: { file: CHAT_MESSAGE_TS, interface: 'ChatMessage' },
    tsLocal: {
      parent: '树形展开：前端按 parent_id 组装的父引用（后端只给扁平列表）',
      children: '树形展开：前端按 parent_id 组装的子列表（后端只给扁平列表）',
    },
  },
  {
    what: 'VDFS 节点',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsNode' },
    ts: { file: VDFS_TS, interface: 'VdfsNode' },
  },
  {
    what: 'VDFS 变更',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsChange' },
    ts: { file: VDFS_TS, interface: 'VdfsChange' },
  },
  {
    what: 'VDFS 访问位',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsAccess' },
    ts: { file: VDFS_TS, interface: 'VdfsAccess' },
  },
  {
    what: 'VDFS 节点内容',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsContent' },
    ts: { file: VDFS_TS, interface: 'VdfsContent' },
  },
  {
    what: 'VDFS 可新建类型',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsNewType' },
    ts: { file: VDFS_TS, interface: 'VdfsNewType' },
  },
  {
    what: 'VDFS 整包导入入口',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsNewImport' },
    ts: { file: VDFS_TS, interface: 'VdfsNewImport' },
  },
  {
    what: 'VDFS 写入响应',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsWriteResponse' },
    ts: { file: VDFS_TS, interface: 'VdfsWriteResponse' },
  },
  {
    what: 'VDFS 字段错误',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsFieldError' },
    ts: { file: VDFS_TS, interface: 'VdfsFieldError' },
  },
  {
    what: 'VDFS 校验错误',
    rust: { file: VDFS_PROVIDER_RS, struct: 'VdfsValidationError' },
    ts: { file: VDFS_TS, interface: 'VdfsValidationError' },
  },
  {
    what: '详情条件谓词',
    rust: { file: DETAIL_RS, struct: 'DetailCondition' },
    ts: { file: FORM_TS, interface: 'DetailCondition' },
  },
  {
    what: '详情选项',
    rust: { file: DETAIL_RS, struct: 'DetailOption' },
    ts: { file: FORM_TS, interface: 'DetailOption' },
  },
  {
    what: '详情表单字段',
    rust: { file: DETAIL_RS, struct: 'DetailField' },
    ts: { file: FORM_TS, interface: 'DetailField' },
  },
  {
    what: '详情分区',
    rust: { file: DETAIL_RS, struct: 'DetailSection' },
    ts: { file: FORM_TS, interface: 'DetailSection' },
  },
  {
    what: '详情预设项',
    rust: { file: DETAIL_RS, struct: 'DetailPreset' },
    ts: { file: FORM_TS, interface: 'DetailPreset' },
  },
  {
    what: '详情预设联动',
    rust: { file: DETAIL_RS, struct: 'DetailPresetSpec' },
    ts: { file: FORM_TS, interface: 'DetailPresetSpec' },
  },
  {
    what: '详情徽标',
    rust: { file: DETAIL_RS, struct: 'DetailBadge' },
    ts: { file: FORM_TS, interface: 'DetailBadge' },
  },
  {
    what: '详情动作',
    rust: { file: DETAIL_RS, struct: 'DetailAction' },
    ts: { file: FORM_TS, interface: 'DetailAction' },
  },
  {
    what: '详情页定义',
    rust: { file: DETAIL_RS, struct: 'DetailDefinition' },
    ts: { file: FORM_TS, interface: 'DetailDefinition' },
  },
]

// ==================== 提取 ====================

/** 后端：全部 `pub const NAME: &str = "v"`（含 `pub(crate)`） */
function rustConsts(src) {
  const out = new Map()
  const re = /(?:pub|pub\(crate\))\s+const\s+(\w+)\s*:\s*&str\s*=\s*"([^"]*)"/g
  let m
  while ((m = re.exec(src))) out.set(m[1], m[2])
  return out
}

/**
 * 后端：按前缀取一组 `pub const PREFIX…: &str = "v"` 的**取值**（不要名字）。
 *
 * 与 `rustConsts` 的区别是：A 组比的是「同名常量的值」，这里比的是「一组常量的
 * 取值集合」——名字对不上无所谓（前端词表本来就不与后端常量同名，如
 * `DETAIL_PICK_*` ↔ `DETAIL_PICKS`），要的是集合相等。
 */
function rustConstGroup(src, prefix) {
  const re = new RegExp(
    `(?:pub|pub\\(crate\\))\\s+const\\s+(${prefix}\\w*)\\s*:\\s*&str\\s*=\\s*"([^"]*)"`,
    'g',
  )
  const out = []
  let m
  while ((m = re.exec(src))) out.push(m[2])
  return out.length > 0 ? out : null
}

/**
 * 前端：全部 `export const NAME = 'v'`。
 *
 * 右侧**必须**是字面量：`export const VDFS_STATUS_FAILED = MESSAGE_STATUS_FAILED`
 * 这类别名不算镜像——它没有第二份真相，值随被别名者走。别名正是**消除**镜像的
 * 手段，不该被当成镜像来守（也因此 `VDFS_STATUS_*` 里只有字面量的两条进 A 组，
 * 其余六条由 C 组经 `MessageStatus` 覆盖）。
 */
function tsConsts(src) {
  const out = new Map()
  const re = /export\s+const\s+(\w+)\s*=\s*'([^']*)'/g
  let m
  while ((m = re.exec(src))) out.set(m[1], m[2])
  return out
}

/** 从 `from` 起第一个 `{` 到配对 `}` 之间的内容（不含花括号本身） */
function braceBlock(src, from) {
  const start = src.indexOf('{', from)
  if (start < 0) return null
  let depth = 0
  for (let i = start; i < src.length; i++) {
    if (src[i] === '{') depth++
    else if (src[i] === '}') {
      depth--
      if (depth === 0) return src.slice(start + 1, i)
    }
  }
  return null
}

/** 找到 `pub enum <name>` 的位置，没有则 null */
function enumHead(src, name) {
  return src.match(new RegExp(`(?:pub|pub\\(crate\\))\\s+enum\\s+${name}\\b`))
}

/**
 * 闭集枚举的变体名。
 *
 * 只认「行首（去空白）大写字母开头、后跟 `,` / `{` / `(`」的行——属性行
 * （`#[default]` / `#[serde(..)]`）与注释行因此自动跳过。枚举体内不会有别的
 * 以大写字母开头的语句，所以不需要 AST。
 */
function rustEnumVariants(src, name) {
  const m = enumHead(src, name)
  if (!m) return null
  const body = braceBlock(src, m.index)
  if (body === null) return null
  const out = []
  for (const raw of body.split(/\r?\n/)) {
    const t = raw.trim()
    if (!t || t.startsWith('#') || t.startsWith('//')) continue
    const vm = t.match(/^([A-Z][A-Za-z0-9]*)\s*(?:,|\{|\()/)
    if (vm) out.push(vm[1])
  }
  return out
}

/**
 * 枚举声明**正上方**的连续属性里，`rename_all` 的取值；没有则 `null`。
 *
 * 从 `enum` 往前逐行收集 `#[...]`，遇到第一个非属性、非空、非注释行即停——
 * 不能只往上看固定字符数，那会跨到上一个枚举的属性上去。
 */
function renameAllOf(src, name) {
  const m = enumHead(src, name)
  if (!m) return null
  for (const a of declAttrs(src, m.index)) {
    const r = a.match(/rename_all\s*=\s*"([^"]+)"/)
    if (r) return r[1]
  }
  return null
}

/**
 * 变体名 → 线格式词，按 `rename_all` 的取值分派。
 *
 * 只支持本仓库**实际用到**的两种。其余取值**报错**而不是猜一个转换规则——
 * 猜错的表现是「看起来正确而实际全错」，那正是这条守卫要防的。
 *
 * 注意 `snake_case` 与 `lowercase` 对**多词**变体结果不同（`ReadOnly` →
 * `read_only` vs `readonly`）；对全单词的枚举两者恰好一致。因为比对的对象是
 * 前端词表，任何**改变线格式词**的属性改动都会被逐词比对抓出来。
 */
const CASE_CONVERTERS = {
  snake_case: (n) =>
    n
      .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
      .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
      .toLowerCase(),
  lowercase: (n) => n.toLowerCase(),
}

/**
 * 前端词表数组的**取值**。
 *
 * 元素允许两种形态：
 * - **本文件的字符串常量名**（`CHAT_ROLE_USER`）——适用于「这个词还要在别处按名
 *   引用」的情形（组件里写 `role === CHAT_ROLE_ASSISTANT`，名字才有价值）；
 * - **裸字符串字面量**（`'invoke'`）——适用于只在词表里出现一次的词。给这类词
 *   硬造一个常量名，只会得到一层**无人引用的间接**（守卫要的是"取值只有一处"，
 *   不是"每个词都有名字"）。
 *
 * 两者之外一律报错（如漏了引号的 `invoke`）：那既不是常量也不是字面量，说明写错了。
 */
function tsArrayValues(src, name) {
  const m = src.match(new RegExp(`const\\s+${name}\\s*=\\s*\\[([\\s\\S]*?)\\]\\s*as\\s+const`))
  if (!m) return { values: null, problem: `未找到词表数组 ${name}` }
  const consts = tsConsts(src)
  const values = []
  for (const raw of m[1].split(',')) {
    const id = raw.split('//')[0].trim()
    if (!id) continue
    const lit = id.match(/^'([^']*)'$/)
    if (lit) {
      values.push(lit[1])
      continue
    }
    if (!consts.has(id)) {
      return {
        values: null,
        problem: `${name} 的元素 ${id} 既不是本文件的字符串常量，也不是字符串字面量`,
      }
    }
    values.push(consts.get(id))
  }
  if (values.length === 0) return { values: null, problem: `词表数组 ${name} 是空的` }
  return { values, problem: null }
}

/** 声明（enum / struct）**正上方**的连续属性行 */
function declAttrs(src, idx) {
  const lines = src.slice(0, idx).split(/\r?\n/)
  const attrs = []
  for (let i = lines.length - 1; i >= 0; i--) {
    const t = lines[i].trim()
    if (t === '' || t.startsWith('//')) continue
    if (t.startsWith('#[')) {
      attrs.push(t)
      continue
    }
    break
  }
  return attrs
}

/**
 * 后端结构体的**线格式字段名**。
 *
 * 三个 serde 属性会改变"字段名是否出现在 JSON 里"，必须处理：
 * - `rename = "x"`            → 线格式名是 `x`（如 `msg_type` → `type`）；
 * - `skip` / `skip_serializing` → **不下发**，字段名不存在。
 *   注意 `skip_serializing_if` 是**条件**跳过（有值时照发），不算；
 * - `flatten`                 → 内层 map 的键展开到外层，**字段名本身不存在**
 *   （如 `VdfsNode::attributes`，前端用索引签名表达同一件事）。
 *
 * 结构体级的 `rename_all` 会整体改写字段名，本检查器不处理——**报错**而不是
 * 默默算错（与 C 组把 `rename_all` 声明本身列为检查项同一立场）。
 */
function rustStructFields(src, name) {
  const m = src.match(new RegExp(`(?:pub|pub\\(crate\\))\\s+struct\\s+${name}\\b`))
  if (!m) return { fields: null, problem: `后端未找到结构体 ${name}` }
  if (declAttrs(src, m.index).some((a) => /rename_all\s*=/.test(a))) {
    return { fields: null, problem: `${name} 用了结构体级 rename_all，本检查器不处理` }
  }
  const body = braceBlock(src, m.index)
  if (body === null) return { fields: null, problem: `${name} 的结构体体解析失败` }

  const out = []
  let drop = false
  let rename = null
  for (const raw of body.split(/\r?\n/)) {
    const t = raw.trim()
    if (!t || t.startsWith('//')) continue
    if (t.startsWith('#[')) {
      // `\bskip_serializing\b(?!_)` 才不会把 `skip_serializing_if` 误判成跳过
      if (/\bskip\b|\bskip_serializing\b(?!_)|flatten\b/.test(t)) drop = true
      const r = t.match(/\brename\s*=\s*"([^"]+)"/)
      if (r) rename = r[1]
      continue
    }
    const fm = t.match(/^(?:pub\s+)?(\w+)\s*:/)
    if (!fm) continue
    if (!drop) out.push(rename ?? fm[1])
    drop = false
    rename = null
  }
  return { fields: out, problem: null }
}

/**
 * 前端接口的字段名。
 *
 * 索引签名（`[k: string]: unknown`）与方法签名（`f(): void`）都不匹配字段正则，
 * 自动跳过——前者正是"其余键 flatten 到顶层"的写法，与后端 `#[serde(flatten)]`
 * 对位，本就不该当成一个字段。
 */
function tsInterfaceFields(src, name) {
  const m = src.match(new RegExp(`(?:export\\s+)?interface\\s+${name}\\b`))
  if (!m) return { fields: null, problem: `前端未找到接口 ${name}` }
  const body = braceBlock(src, m.index)
  if (body === null) return { fields: null, problem: `${name} 的接口体解析失败` }
  const out = []
  for (const raw of body.split(/\r?\n/)) {
    const t = raw.trim()
    if (!t || t.startsWith('//') || t.startsWith('*') || t.startsWith('/*')) continue
    const fm = t.match(/^([A-Za-z_$][\w$]*)\??\s*:/)
    if (fm) out.push(fm[1])
  }
  return { fields: out, problem: null }
}

/**
 * 递归列出目录下的 .ts / .vue 文件。
 *
 * **排除 `__tests__`**：与 mechanism-audit 的既有口径一致——测试持有的是
 * **协议夹具**（把地址钉成字面量正是它的职责），生产代码才不许持有。
 */
function walk(dir) {
  const out = []
  if (!fs.existsSync(dir)) return out
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, e.name)
    if (e.isDirectory()) {
      if (e.name === '__tests__') continue
      out.push(...walk(abs))
    } else if (/\.(ts|vue)$/.test(e.name)) {
      out.push(abs)
    }
  }
  return out
}

/** 后端侧：全部 `*.rs`（E 组扫跨栈导航头用） */
function walkRs(dir) {
  const out = []
  if (!fs.existsSync(dir)) return out
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, e.name)
    if (e.isDirectory()) out.push(...walkRs(abs))
    else if (e.name.endsWith('.rs')) out.push(abs)
  }
  return out
}

// ==================== 主流程 ====================

let errors = 0
let headerCount = 0
let headerBad = 0

const readIfExists = (relPath) => {
  const abs = path.join(REPO, relPath)
  return fs.existsSync(abs) ? fs.readFileSync(abs, 'utf8') : null
}
const relOf = (abs) => path.relative(REPO, abs).replace(/\\/g, '/')

console.log('=== protocol-mirror-audit：跨栈协议常量 ===')
console.log(dim(`    仓库根：${REPO}`))
console.log()

// ---------- A ----------
console.log('--- A. 常量镜像（自动发现同名 `VDFS_*`，逐字比对）---')

const rustAll = new Map()
for (const f of RUST_CONST_FILES) {
  const src = readIfExists(f)
  if (src === null) {
    errors++
    console.log(`  ${red(BAD)} 后端常量源不存在：${f}`)
    continue
  }
  for (const [k, v] of rustConsts(src)) rustAll.set(k, { value: v, file: f })
}

const tsAll = new Map()
for (const f of TS_CONST_FILES) {
  const src = readIfExists(f)
  if (src === null) {
    errors++
    console.log(`  ${red(BAD)} 前端常量源不存在：${f}`)
    continue
  }
  for (const [k, v] of tsConsts(src)) tsAll.set(k, { value: v, file: f })
}

let mirrorCount = 0
for (const [name, ts] of tsAll) {
  if (!name.startsWith('VDFS_')) continue
  if (LOCAL_ONLY.some((l) => l.name === name)) continue

  const alias = ALIASES.find((a) => a.ts === name)
  let rustVal = null
  let rustWhere = ''

  if (rustAll.has(name)) {
    rustVal = rustAll.get(name).value
    rustWhere = `${rustAll.get(name).file}::${name}`
  } else if (alias) {
    const src = readIfExists(alias.rust.file)
    const hit = src === null ? null : (rustConsts(src).get(alias.rust.name) ?? null)
    rustWhere = `${alias.rust.file}::${alias.rust.name}`
    if (hit === null) {
      errors++
      console.log(`  ${red(BAD)} [镜像] ${name} —— 别名指向的后端常量未找到`)
      console.log(dim(`      ${rustWhere}`))
      continue
    }
    rustVal = hit
  } else {
    errors++
    console.log(`  ${red(BAD)} [镜像] ${name} —— 前端持有，后端无同名常量，也未登记`)
    console.log(dim('      它确是协议词镜像（只是名字不同）→ 登记到 ALIASES；'))
    console.log(dim('      它是前端自持概念 → 登记到 LOCAL_ONLY 并写明理由。'))
    continue
  }

  mirrorCount++
  if (rustVal === ts.value) {
    const via = alias ? dim(`（后端 ${alias.rust.name}）`) : ''
    console.log(`  ${green(OK)} ${name} = ${JSON.stringify(rustVal)}${via}`)
    continue
  }
  errors++
  console.log(
    `  ${red(BAD)} ${name} —— 取值不一致：后端 ${JSON.stringify(rustVal)} ≠ 前端 '${ts.value}'`,
  )
  console.log(dim(`      ${rustWhere} ↔ ${ts.file}::${name}`))
}

// ---------- B ----------
console.log()
console.log('--- B. 缺席检查（前端不得再持有会话地址段）---')
const frontendFiles = walk(path.join(REPO, 'tauri', 'src'))
for (const a of ABSENT) {
  const hits = []
  for (const f of frontendFiles) {
    const lines = fs.readFileSync(f, 'utf8').split(/\r?\n/)
    lines.forEach((line, i) => {
      if (a.pattern.test(line)) hits.push(`${relOf(f)}:${i + 1}`)
    })
  }
  if (hits.length === 0) {
    console.log(`  ${green(OK)} ${a.name} —— ${a.what}`)
    continue
  }
  errors += hits.length
  console.log(`  ${red(BAD)} ${a.name} —— 出现 ${hits.length} 处（${a.what}）`)
  for (const h of hits.slice(0, 10)) console.log(`      ${red(h)}`)
  console.log(dim('      这两个段名由后端 provider 决定，前端应运行期发现'))
}

// ---------- C ----------
console.log()
console.log('--- C. 闭集词表（后端枚举取值 ↔ 前端词表，集合相等）---')
for (const e of ENUM_SETS) {
  const rustSrc = readIfExists(e.rust.file)
  const tsSrc = readIfExists(e.ts.file)
  const problems = []
  let rustWords = null
  let tsWords = null

  // 后端侧的两种写法：枚举（需按 rename_all 转换）/ 常量组（字面即取值）
  const rustLabel = e.rust.enum ?? `${e.rust.constPrefix}*`

  if (rustSrc === null) {
    problems.push(`后端文件不存在：${e.rust.file}`)
  } else if (e.rust.enum) {
    const variants = rustEnumVariants(rustSrc, e.rust.enum)
    if (variants === null) {
      problems.push(`后端未找到枚举 ${e.rust.enum}`)
    } else {
      const style = renameAllOf(rustSrc, e.rust.enum)
      const convert = style === null ? null : CASE_CONVERTERS[style]
      if (style === null) {
        problems.push(`${e.rust.enum} 缺 #[serde(rename_all = "…")] 声明`)
      } else if (!convert) {
        problems.push(
          `${e.rust.enum} 的 rename_all = "${style}" 本检查器不支持` +
            `（只支持 ${Object.keys(CASE_CONVERTERS).join(' / ')}）`,
        )
      } else {
        rustWords = variants.map(convert)
      }
    }
  } else if (e.rust.constPrefix) {
    const values = rustConstGroup(rustSrc, e.rust.constPrefix)
    if (values === null) problems.push(`后端未找到常量组 ${e.rust.constPrefix}*`)
    else rustWords = values
  } else {
    problems.push('登记项既没写 enum 也没写 constPrefix —— 无法判断后端取值')
  }

  if (tsSrc === null) {
    problems.push(`前端文件不存在：${e.ts.file}`)
  } else {
    const got = tsArrayValues(tsSrc, e.ts.array)
    if (got.problem) problems.push(got.problem)
    else tsWords = got.values
  }

  if (rustWords !== null && tsWords !== null) {
    const rset = new Set(rustWords)
    const tset = new Set(tsWords)
    const missingInTs = rustWords.filter((w) => !tset.has(w))
    const extraInTs = tsWords.filter((w) => !rset.has(w))
    if (missingInTs.length) problems.push(`前端词表缺少：${missingInTs.join(', ')}`)
    if (extraInTs.length) problems.push(`前端词表多出：${extraInTs.join(', ')}`)
  }

  if (problems.length === 0) {
    console.log(
      `  ${green(OK)} ${rustLabel} ↔ ${e.ts.array} —— ${e.what}（${rustWords.length} 个取值）`,
    )
    continue
  }

  errors += problems.length
  console.log(`  ${red(BAD)} ${rustLabel} ↔ ${e.ts.array} —— ${e.what}`)
  for (const msg of problems) console.log(`      ${red(msg)}`)
  console.log(dim(`      ${e.rust.file} ↔ ${e.ts.file}`))
}

// ---------- D ----------
console.log()
console.log('--- D. 结构体字段（前端字段必须能在后端找到）---')
for (const s of STRUCT_SETS) {
  const rustSrc = readIfExists(s.rust.file)
  const tsSrc = readIfExists(s.ts.file)
  const problems = []
  let rustFields = null
  let tsFields = null

  if (rustSrc === null) {
    problems.push(`后端文件不存在：${s.rust.file}`)
  } else {
    const got = rustStructFields(rustSrc, s.rust.struct)
    if (got.problem) problems.push(got.problem)
    else rustFields = got.fields
  }

  if (tsSrc === null) {
    problems.push(`前端文件不存在：${s.ts.file}`)
  } else {
    const got = tsInterfaceFields(tsSrc, s.ts.interface)
    if (got.problem) problems.push(got.problem)
    else tsFields = got.fields
  }

  if (rustFields !== null && tsFields !== null) {
    const rset = new Set(rustFields)
    const local = s.tsLocal ?? {}
    const extra = tsFields.filter((f) => !rset.has(f) && !local[f])
    if (extra.length) problems.push(`前端持有后端不存在的字段：${extra.join(', ')}`)
  }

  if (problems.length === 0) {
    const localCount = Object.keys(s.tsLocal ?? {}).length
    const via = localCount ? dim(`（另含 ${localCount} 个前端自持）`) : ''
    console.log(
      `  ${green(OK)} ${s.rust.struct} ↔ ${s.ts.interface} —— ${s.what}` +
        `（后端 ${rustFields.length} / 前端 ${tsFields.length} 字段）${via}`,
    )
    continue
  }

  errors += problems.length
  console.log(`  ${red(BAD)} ${s.rust.struct} ↔ ${s.ts.interface} —— ${s.what}`)
  for (const msg of problems) console.log(`      ${red(msg)}`)
  console.log(dim(`      ${s.rust.file} ↔ ${s.ts.file}`))
  console.log(dim('      三条出路：后端换了名字 → 改前端；前端自持 → 登记本对的 tsLocal；'))
  console.log(dim('      前端压根没用它 → 删掉该字段（类型里的死字段无人看守）。'))
}

// ==================== E. 跨栈导航头 ====================

/**
 * `// Corresponding Frontend: <相对仓库根的路径>` 必须指向**真实存在**的文件。
 *
 * 这条头是跨栈导航的入口（从后端协议文件跳到它的前端镜像），而它腐烂的方式很安静：
 * 前端文件改名 / 删除后，头照旧指向旧路径，**没人会被告知**——实测 8 条里 **7 条**
 * 悬空，其中 6 条指向 `tauri/src/protocols/`，而那个目录**从未在版本史里出现过**
 * （`git log --diff-filter=A -- 'tauri/src/protocols/*'` 为空）。
 *
 * 约定因此收紧为：**写了就必须指向真实文件；前端没有镜像就别写**。写一条假指针
 * 比不写更糟——不写只是缺个跳转，写假的会让人以为那边有人在看。
 */
const HEADER_RE = /^\s*(?:\/\/!|\/\/)\s*Corresponding Frontend:\s*(\S+)\s*$/

// ---------- E 组 ----------
console.log()
console.log('E. 跨栈导航头（Corresponding Frontend）')
{
  const headers = []
  for (const file of walkRs(path.join(REPO, 'symbio', 'src'))) {
    const rel = path.relative(REPO, file).split(path.sep).join('/')
    const lines = fs.readFileSync(file, 'utf8').split('\n')
    lines.forEach((line, i) => {
      const m = line.match(HEADER_RE)
      if (m) headers.push({ rel, target: m[1], line: i + 1 })
    })
  }

  let bad = 0
  for (const h of headers) {
    if (fs.existsSync(path.join(REPO, h.target))) {
      console.log(`  ${green(OK)} ${h.rel}:${h.line} → ${h.target}`)
    } else {
      bad += 1
      errors += 1
      console.log(`  ${red(BAD)} ${h.rel}:${h.line} → ${h.target}（目标不存在）`)
      console.log(dim('      前端改名/删除后这条头就悬空了；没有对应镜像就把这行删掉（假指针比没有更糟）。'))
    }
  }
  if (headers.length === 0) console.log(dim('  （没有任何文件声明跨栈对应）'))
  headerCount = headers.length
  headerBad = bad
}

// ---------- 收尾 ----------
console.log()
console.log(`Errors: ${errors}`)

if (errors > 0) {
  console.log()
  console.log(
    yellow(
      '  A 组：前端持有的 VDFS_* 常量必须与后端同名常量逐字相等（改后端就要改前端镜像）。\n' +
        '  B 组：会话地址段不该出现在前端——它们是运行期发现的数据，不是常量。\n' +
        '  C 组：后端闭集（枚举或常量组）的取值集合必须与前端词表相等。\n' +
        '  D 组：前端接口的字段必须能在后端结构体里找到（改名会让前端静默读到 undefined）。\n' +
        '  E 组：Corresponding Frontend 头必须指向真实存在的文件（悬空就删掉那行）。',
    ),
  )
  process.exit(1)
}

console.log(
  green(
    `  A 组 ${mirrorCount} 条常量镜像 + B 组 ${ABSENT.length} 项缺席检查 + ` +
      `C 组 ${ENUM_SETS.length} 张闭集词表 + D 组 ${STRUCT_SETS.length} 对结构体字段 + ` +
      `E 组 ${headerCount} 条跨栈导航头，全部一致` +
      `（扫描 ${frontendFiles.length} 个前端文件）`,
  ),
)
process.exit(0)
