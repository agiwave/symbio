/**
 * test-fixtures-rs —— core 测试夹具的事件拼写与声明表投影
 *
 * ## 为什么要有这一份
 *
 * `symbio-sdk` 在 `symbio-plugin-api` 的下层（api 依赖 sdk），core 的用例因此不能
 * 直接 import `symbio_plugin_api::events`。此前 `crates/symbio-sdk/src/test_fixtures.rs`
 * 把这批事件名**手抄**了一遍：值一改就要改两处，而 SV-001 只扫插件 crate，看不见
 * 这一层（D-392）。
 *
 * 现在夹具的**选面**（用哪些名字）仍由用例决定（下面的 `FIXTURE_EVENTS`），但每一条
 * **定义**（值 / 格位 / 角色 / host）都从 owner 的 `EventDecl` 投影——与
 * `crates/symbio-plugin-api/src/events.rs` 同一个读法（[`pluginDeclarationFaces`]）。
 * 同一份 owner 声明生成两处产物：词汇层给生产读者，本产物给 core 的测试读者。
 * 手改本产物会被 `gen-facts --check` 判红。
 *
 * ## 为什么不把夹具搬出 sdk
 *
 * 夹具只依赖 `symbio-kernel`（sdk 的下层）。把它放进独立测试 crate 会让 sdk 的
 * `#[cfg(test)]` 用例反向依赖一个上层 crate（或形成 dev-dependency 环）；而生产代码
 * 里一份副本都不留的前提已经由 `#[cfg(any(test, feature = "test-fixtures"))]` 满足。
 */
// @gate lib
import { pluginDeclarationFaces } from './events-rs.mjs'

const OUT = 'crates/symbio-sdk/src/test_fixtures.rs'

/**
 * 夹具的**消费面**：core 用例实际构造 / 断言的那些事件名。
 *
 * 这里只写「要哪些名字」，**不写值、格位或角色**——那三项逐条从 owner 的声明取。
 */
const GROUPS = [
  {
    title: '对话面（`turn`）',
    names: [
      'user.message',
      'chat.assistant.reported',
      'chat.assistant.opening',
      'chat.assistant.final',
      'chat.assistant.fallback',
      'chat.assistant.progress',
      'turn.branch',
      'turn.branch_selected',
      'model.speech.denied',
    ],
  },
  {
    title: '工具派发面',
    names: ['tool.requested'],
  },
  {
    title: '产物与消息面',
    names: [
      'artifact.added',
      'message.added',
      'message.edited',
      'message.removed',
      'session.consolidated',
    ],
  },
  {
    title: '承诺面',
    names: [
      'commitment.opened',
      'commitment.released',
      'commitment.broken',
      'commitment.asserted',
    ],
  },
  { title: '裁决面', names: ['verdict.asserted'] },
]

/** 协议五格：名字是内核语法常量，格位在内核里固定为 `(Scope, Asserted)`。 */
const PROTOCOL = [
  ['EVENT_REQUEST', 'request'],
  ['EVENT_SENT', 'sent'],
  ['EVENT_RESPONSE', 'response'],
  ['EVENT_FAILED', 'failed'],
  ['EVENT_RETRY', 'retry'],
]

const constOf = (name) => `EVENT_${name.replace(/\./g, '_').toUpperCase()}`

/** 名字 → 声明（名字必须由某个 owner 提交过 `EventDecl`，否则抛错）。 */
function declarations(root) {
  const faces = pluginDeclarationFaces(root)
  const byName = new Map()
  for (const [owner, face] of faces) {
    for (const [name, cell] of face.cells) byName.set(name, { name, owner, ...cell })
  }
  const groups = []
  for (const group of GROUPS) {
    const rows = []
    for (const name of group.names) {
      const row = byName.get(name)
      if (!row) {
        throw new Error(
          `test-fixtures-rs：夹具选了 ${JSON.stringify(name)}，但没有任何插件为它提交 ` +
            '`EventDecl`——没有 owner 的名字不能进测试夹具（D-392）',
        )
      }
      if (!row.entity || !row.verb) {
        throw new Error(`test-fixtures-rs：${name} 的格位读不出（owner ${row.owner}）`)
      }
      rows.push(row)
    }
    groups.push({ title: group.title, rows })
  }
  return groups
}

const declLine = (r) => {
  const roles = r.roles ?? []
  const host = r.host === true
  const ctor = host ? (roles.length ? 'host_with_roles' : 'host') : roles.length ? 'with_roles' : 'new'
  const roleArg = roles.length
    ? `, &[${roles.map((x) => `FactRole::${x}`).join(', ')}]`
    : ''
  return `EventDecl::${ctor}(${constOf(r.name)}, (Entity::${r.entity}, Verb::${r.verb})${roleArg})`
}

/** 纯函数：owner 声明 → `test_fixtures.rs` 全文。 */
export function render({ root }) {
  const groups = declarations(root)
  const rows = groups.flatMap((g) => g.rows)
  const L = [
    '//! core 用例的事件拼写与三张词汇表 —— **由 `scripts/facts/test-fixtures-rs.mjs` 生成，请勿手改**。',
    '//!',
    '//! 事实源：各插件 owner 提交的 `EventDecl`（值 / 格位 / 角色 / host）。同一个源还生成',
    '//! `symbio-plugin-api/src/events.rs`（给生产读者）；本产物只服务 core 的测试读者。',
    '//!',
    '//! ## 为什么它在 core 里，却不是 core 的词典',
    '//!',
    '//! D-203 裁决三把业务事件名移出内核：名字只由 owner 声明，core 的折叠经**构造期形参**',
    '//! （[`ChatVocab`] / [`MessageVocab`] / [`CommitmentVocab`]）收下拼写，一行都不读。',
    '//! 生产拼写来自 `symbio_plugin_api::events`；core 不能依赖那个 crate——它反过来依赖 core。',
    '//! 所以夹具的拼写由生成器从**同一批 owner 声明**投影一份，只在 `#[cfg(test)]` /',
    '//! `feature = "test-fixtures"` 下存在。**测试也是调用方**，它交的拼写就是它要断言的语义。',
    '//!',
    '//! **本产物只供 `#[cfg(test)]` / `feature = "test-fixtures"` 使用，不进生产构建。**',
    '//! 生产拼写只有一个出口——`symbio_plugin_api::events`；若把本生成器改成「顺带产出生产',
    '//! 常量」，它就成了第二词汇面，与 D-392 的单一源前提直接冲突。',
    '//!',
    '//! 删掉本模块不影响任何生产代码；它也不参与 CS-005 / CS-006 / CS-007 的判定（那三条',
    '//! 判的是内核名字表与清单的消费方分布）。',
    '//!',
    '//! ## 为什么还要一张 `EVENTS`',
    '//!',
    '//! core 的不变量判据（`is_stop_fact` / `check_all` / `closure_reason_closed`）的词汇面是',
    '//! `&[EventDecl]`——它们问「这条名字扮演什么角色」，答案必须来自一张声明表。这张表由',
    '//! 生成器按 owner 声明逐条投影；协议五格仍取内核常量（它们是语法闭集的一部分）。',
    '',
    'use crate::event::{EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT};',
    'use crate::projection::vocab::{ChatVocab, CommitmentVocab, MessageVocab};',
    'use crate::{Entity, EventDecl, FactRole, Verb};',
    '',
  ]

  for (const group of groups) {
    L.push(`// ── ${group.title} ──`)
    for (const row of group.rows) L.push(`pub const ${constOf(row.name)}: &str = "${row.name}";`)
    L.push('')
  }

  L.push('/// 用例交出去的声明表：格位与角色与各 owner 的 `EventDecl` 逐条相同。')
  L.push('///')
  L.push('/// 三档角色必须有人住（判据 `CS-011` 判整仓：插件生产声明里缺一档即红）：')
  L.push('/// 预算声明在开轮 / 派发，收束在定稿，停机在被拒。夹具表照抄这三处，否则守卫判据会在「角色没人住」上红。')
  L.push('#[rustfmt::skip]')
  L.push('pub const EVENTS: &[EventDecl] = &[')
  L.push('    // 协议五格：名字与格位是内核语法的一部分，夹具照内核常量走。')
  for (const [constName, value] of PROTOCOL) {
    L.push(`    EventDecl::new(${constName}, (Entity::Scope, Verb::Asserted)), // ${value}`)
  }
  for (const row of rows) L.push(`    ${declLine(row)}, // ${row.name} ← ${row.owner}`)
  L.push('];')
  L.push('')

  L.push('/// 对话发言面（8 条事件 + 1 个端口名）。')
  L.push('pub const CHAT: ChatVocab = ChatVocab {')
  for (const [field, constName] of [
    ['user_message', 'EVENT_USER_MESSAGE'],
    ['branch', 'EVENT_TURN_BRANCH'],
    ['branch_selected', 'EVENT_TURN_BRANCH_SELECTED'],
    ['reported', 'EVENT_CHAT_ASSISTANT_REPORTED'],
    ['opening', 'EVENT_CHAT_ASSISTANT_OPENING'],
    ['final_reply', 'EVENT_CHAT_ASSISTANT_FINAL'],
    ['fallback', 'EVENT_CHAT_ASSISTANT_FALLBACK'],
    ['progress', 'EVENT_CHAT_ASSISTANT_PROGRESS'],
  ]) {
    L.push(`    ${field}: ${constName},`)
  }
  L.push('    model_port: "model",')
  L.push('};')
  L.push('')

  L.push('/// 消息变更面（6 条）。')
  L.push('pub const MESSAGE: MessageVocab = MessageVocab {')
  for (const [field, constName] of [
    ['tool_requested', 'EVENT_TOOL_REQUESTED'],
    ['artifact_added', 'EVENT_ARTIFACT_ADDED'],
    ['message_added', 'EVENT_MESSAGE_ADDED'],
    ['message_edited', 'EVENT_MESSAGE_EDITED'],
    ['message_removed', 'EVENT_MESSAGE_REMOVED'],
    ['consolidated', 'EVENT_SESSION_CONSOLIDATED'],
  ]) {
    L.push(`    ${field}: ${constName},`)
  }
  L.push('};')
  L.push('')

  L.push('/// 承诺面（4 条）。')
  L.push('pub const COMMITMENT: CommitmentVocab = CommitmentVocab {')
  for (const [field, constName] of [
    ['opened', 'EVENT_COMMITMENT_OPENED'],
    ['released', 'EVENT_COMMITMENT_RELEASED'],
    ['broken', 'EVENT_COMMITMENT_BROKEN'],
    ['asserted', 'EVENT_COMMITMENT_ASSERTED'],
  ]) {
    L.push(`    ${field}: ${constName},`)
  }
  L.push('};')

  return `${L.join('\n').replace(/\n+$/, '')}\n`
}

export default {
  id: 'test-fixtures-rs',
  title: 'core 测试夹具的事件拼写与声明表（从 owner 的 `EventDecl` 投影）',
  out: OUT,
  stamp: false,
  render,
}
