/**
 * plugin-entry-audit 回归测试
 *
 * 一个只会亮绿灯的守卫等于没有守卫。这里对每条规则都注入一个**真实违规**，
 * 断言脚本确实变红；再对同一段文本放进注释 / 放进测试文件 / 加上带理由的豁免注释，
 * 断言它不再报（否则守卫会因为误报被人用豁免喂到失效）。
 *
 * 跑法：node --test scripts/plugin-entry-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./plugin-entry-audit.mjs', import.meta.url))

// 规则条数**从脚本自己推导**，不手写。
// 原先这里写死 `/9 条规则全部通过/`，而紧邻的注释却声称「条数不写死：规则表是唯一
// 真相源，脚本从它推导」——注释与代码说的是两件事。后果：加一条规则（E-010）就让
// 这条测试红，而它红的原因是**数字过时**，不是规则坏了；于是下一个人会去改数字，
// 顺手把「规则集不能缩小」这层意思也一起改掉。派生出来的数字两头都对。
const DECLARED_RULES = [...fs.readFileSync(script, 'utf8').matchAll(/^ {2}'(E-\d+)':/gm)].map(
  (m) => m[1],
)

/**
 * 在临时目录里搭一棵最小仓库树并跑审计。
 *
 * @param {Record<string, string>} files 相对仓库根的路径 → 内容
 * @param {{strict?: boolean}} [opts]
 */
function audit(files, { strict = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'plugin-entry-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(
      process.execPath,
      [script, `--root=${root}`, ...(strict ? ['--strict'] : [])],
      { env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 20000 },
    )
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

/** `symbio_core` 的最小事实源：工厂 id、协议端点、路由常量 */
const CORE = {
  'symbio/src/symbio_core/keys/ids.rs': `pub const PLUGIN_SESSION: &str = "session";\n`,
  'symbio/src/symbio_core/mod.rs':
    `pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";\n`,
  'symbio/src/symbio_core/capability/option.rs':
    `pub const TRAVERSE_AVAILABLE_OPTIONS: &str = "available_options";\n`,
  'symbio/src/symbio_core/keys/paths.rs': `pub const SESSION_CHAT_SEND: &str = "session/chat/send";\n`,
}

/** 一个干净的最小插件：两条静态路由臂 + 只认 `available_tools` 的 traverse */
const PLUGIN = `use crate::symbio_core::{PluginError, PluginMeta, PATH, PLUGIN_SESSION, TRAVERSE_AVAILABLE_TOOLS};

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new(PLUGIN_SESSION, "会话管理")
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        match path.as_str() {
            "chat/send" => Ok(PluginPayload::new(&1)),
            "update" => Ok(PluginPayload::new(&2)),
            _ => Err(PluginError::NotFound(path)),
        }
    }

    async fn traverse(self: Arc<Self>, _path: String, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(sub_path));
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
`

const CLEAN = {
  ...CORE,
  'symbio/src/plugins/session/plugin.rs': PLUGIN,
  'docs/reference/ROUTES.md': '| `session/chat/send` | 发言 |\n',
}

test('干净树通过（exit 0）', () => {
  const r = audit(CLEAN)
  assert.equal(r.status, 0, r.stdout)
  // 条数从脚本的规则表推导（见上方 `DECLARED_RULES`）——手写数字会在加规则时漂
  assert.ok(DECLARED_RULES.length > 0, '规则表没解析出来，判据本身坏了')
  assert.match(r.stdout, new RegExp(`${DECLARED_RULES.length} 条规则全部通过`))
})

test('报告段给出每条路由的消费方计数', () => {
  const r = audit(CLEAN)
  assert.match(r.stdout, /session\/chat\/send\s+refs=1/)
  assert.match(r.stdout, /session\/update\s+refs=0\s+← 无消费方/)
})

// ── E-001：`PluginMeta::new` 首参必须 == 插件目录名 ───────────────────────
test('E-001 命中：meta 首参写成 "sessions" 而目录是 session', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/plugin.rs': PLUGIN.replace('PluginMeta::new(PLUGIN_SESSION', 'PluginMeta::new("sessions"'),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /E-001 .*plugins\/session\/plugin\.rs/)
})

test('E-001 不误报：首参用常量且与目录名一致', () => {
  const r = audit(CLEAN)
  assert.doesNotMatch(r.stdout, /E-001 .*plugin\.rs:\d/)
})

// ── E-002：路径字面量首段必须是插件目录名 ───────────────────────────────
test('E-002 命中：代码里引用 meta id 当命名空间（`sessions/update`）', () => {
  const r = audit({
    ...CLEAN,
    // meta 首参错写成 sessions ⇒ `sessions` 成了一个「幽灵命名空间」
    'symbio/src/plugins/session/plugin.rs': PLUGIN.replace('PluginMeta::new(PLUGIN_SESSION', 'PluginMeta::new("sessions"'),
    'symbio/src/plugins/session/caller.rs':
      `pub fn p() -> String { "sessions/update".to_string() }\n`,
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /E-002 .*caller\.rs:1/)
})

// ── E-003：调用侧不得写字面量 PATH ──────────────────────────────────────
test('E-003 命中：`set(PATH, "字面量")`', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/caller.rs':
      `pub fn p(ctx: Arc<dyn PluginInvokeRequest>) { ctx.set(PATH, "session/chat/send".to_string()); }\n`,
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /E-003 .*caller\.rs:1/)
})

test('E-003 不误报：用常量（`SESSION_CHAT_SEND.to_string()`）', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/caller.rs':
      `pub fn p(ctx: Arc<dyn PluginInvokeRequest>) { ctx.set(PATH, SESSION_CHAT_SEND.to_string()); }\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

test('E-003 豁免：带理由的 `plugin-entry-allow` 不再报', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/caller.rs':
      `// plugin-entry-allow E-003: 这里刻意造一条非法路径验证兜底\npub fn p(ctx: Arc<dyn PluginInvokeRequest>) { ctx.set(PATH, "session/chat/send".to_string()); }\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

test('E-003 豁免：理由为空的 `plugin-entry-allow` 视为未豁免', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/caller.rs':
      `// plugin-entry-allow E-003:\npub fn p(ctx: Arc<dyn PluginInvokeRequest>) { ctx.set(PATH, "session/chat/send".to_string()); }\n`,
  })
  assert.equal(r.status, 1, r.stdout)
})

test('E-003 不误报：测试文件里的假路径不判', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/caller.test.rs':
      `pub fn p(ctx: Arc<dyn PluginInvokeRequest>) { ctx.set(PATH, "work/whatever".to_string()); }\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

// ── E-004：`traverse` 端点必须用常量 ────────────────────────────────────
test('E-004 命中：`sub_path != "available_tools"`', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/session/plugin.rs': PLUGIN.replace(
      'sub_path != TRAVERSE_AVAILABLE_TOOLS',
      'sub_path != "available_tools"',
    ),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /\[ERROR\] E-004 .*plugin\.rs/)
})

test('E-004 不误报：常量定义行本身', () => {
  const r = audit(CLEAN)
  assert.doesNotMatch(r.stdout, /\[ERROR\] E-004/)
})

// ── E-005：路径必须真实存在（WARNING）──────────────────────────────────
test('E-005 命中：`session/chat` 少了 `/send`', () => {
  const r = audit(
    {
      ...CLEAN,
      'symbio/src/plugins/session/caller.rs':
        `pub fn p() -> String { "session/chat".to_string() }\n`,
    },
    { strict: true },
  )
  assert.equal(r.status, 2, r.stdout)
  assert.match(r.stdout, /E-005 .*caller\.rs:1/)
})

test('E-005 不误报：动态命名空间（`local/<工具名>`）', () => {
  const r = audit(
    {
      ...CLEAN,
      'symbio/src/plugins/session/caller.rs':
        `pub fn p() -> String { "local/read_file".to_string() }\n`,
    },
    { strict: true },
  )
  assert.equal(r.status, 0, r.stdout)
})

test('E-005 不误报：常量定义行（它是真相源本身，不是引用）', () => {
  const r = audit(CLEAN, { strict: true })
  assert.equal(r.status, 0, r.stdout)
})

// ── E-006：权威清单里的路径前缀（WARNING）─────────────────────────────
//
// 注意 E-002 / E-006 与 E-001 是**同一个根因的下游**：meta id 写错（E-001）之后，
// 「错名已经扩散到哪」才是要修的范围——E-002 查代码，E-006 查文档。
// 所以这两个用例断言的是**输出行**而不是退出码（退出码由 E-001 决定）。
// 真实案例：`hook` 的 `"hooks"` 只被 E-001 抓到时，你还不知道三处文档已经照抄了它。
test('E-006 命中：ROUTES.md 里写了 `sessions/update`', () => {
  const r = audit(
    {
      ...CLEAN,
      'symbio/src/plugins/session/plugin.rs': PLUGIN.replace(
        'PluginMeta::new(PLUGIN_SESSION',
        'PluginMeta::new("sessions"',
      ),
      'docs/reference/ROUTES.md': '| `sessions/update` | 合并写入 |\n',
    },
    { strict: true },
  )
  assert.match(r.stdout, /\[WARN\]\s+E-006 docs\/reference\/ROUTES\.md:1/)
})

test('E-006 不误报：退役路由的**前缀**仍然合法（`session/append`）', () => {
  const r = audit(
    {
      ...CLEAN,
      'docs/reference/ROUTES.md':
        '> `session/append` **已退役**（与 `vdfs/delete` 共用实现）。\n',
    },
    { strict: true },
  )
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, /\[WARN\]\s+E-006/)
})

test('E-006 不误报：非权威文档不判', () => {
  const r = audit(
    {
      ...CLEAN,
      'symbio/src/plugins/session/plugin.rs': PLUGIN.replace(
        'PluginMeta::new(PLUGIN_SESSION',
        'PluginMeta::new("sessions"',
      ),
      'symbio/src/plugins/session/docs/legacy.md': '历史入口：`sessions/update` 已下线。\n',
    },
    { strict: true },
  )
  assert.doesNotMatch(r.stdout, /\[WARN\]\s+E-006/)
})

// ── 形态识别：不该把动态分发判成死路由 ────────────────────────────────
test('形态识别：按工具名分发（`.find(|t| t.name()`）判为运行期动态', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/local/plugin.rs': `impl Plugin for LocalPlugin {
    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
            return tool.execute(ctx).await;
        }
        Err(PluginError::NotFound(path))
    }
    async fn traverse(self: Arc<Self>, _path: String, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(sub_path));
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
`,
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /local\s+\[运行期动态\]/)
})

test('形态识别：`route` 恒 Err 判为「恒 NotFound」', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/work/plugin.rs': `impl Plugin for WorkPlugin {
    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound(format!("work 无自有协议路由")))
    }
    async fn traverse(self: Arc<Self>, _path: String, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(sub_path));
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
`,
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /work\s+\[恒 NotFound\]/)
})

// ── E-007：插件不得按值持有兄弟插件实例 ──────────────────────────────────
//
// 违规形态取自真实事件：`telegram` 曾有个 `llm_plugin` 字段按值持有 session 实例，
// 而唯一写入点传的是 `None` ⇒ 整条 LLM 回复链路从未通。测试用 `field` 参数
// 替换那一行，其余部分保持一个干净插件，确保命中的**只**是字段形态。
const siblingPlugin = (field) => `use std::sync::Arc;

pub struct TelegramPlugin {
    ${field}
}

impl Plugin for TelegramPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("telegram", "Telegram 集成")
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        match path.as_str() {
            "send" => Ok(PluginPayload::new(&1)),
            _ => Err(PluginError::NotFound(path)),
        }
    }

    async fn traverse(self: Arc<Self>, _path: String, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(sub_path));
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
`

const withField = (field) => ({
  ...CLEAN,
  'symbio/src/plugins/telegram/plugin.rs': siblingPlugin(field),
})

// 断言一律落在**报错行**上，不用裸 `/E-007/`：汇总段那句
// `✓ E-007  不按值持有兄弟插件` 也含 `E-007`，会把「不误报」的用例喂成假绿。
// （E-006 的第一版就踩过这个坑。）
const E007_HIT = /\[ERROR\]\s+E-007\s+symbio\/src\/plugins\/telegram\/plugin\.rs:\d/

test('E-007 命中：`Arc<RwLock<Option<Arc<dyn Plugin>>>>` 字段（telegram 的历史形态）', () => {
  const r = audit(withField('llm_plugin: Arc<RwLock<Option<Arc<dyn Plugin>>>>,'))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E007_HIT)
})

test('E-007 命中：直接 `Option<Arc<dyn Plugin>>` 字段（没有 RwLock 包一层）', () => {
  const r = audit(withField('llm_plugin: Option<Arc<dyn Plugin>>,'))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E007_HIT)
})

test('E-007 不误报：`Weak`（向上引用父）', () => {
  const r = audit(withField('parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,'))
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E007_HIT)
})

test('E-007 不误报：容器按名持有多个子实例（HashMap）', () => {
  const r = audit(withField('instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,'))
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E007_HIT)
})

test('E-007 不误报：字段名是 parent / router（本仓专指向上引用）', () => {
  for (const field of ['parent: Option<Arc<dyn Plugin>>,', 'router: Option<Arc<dyn Plugin>>,']) {
    const r = audit(withField(field))
    assert.equal(r.status, 0, `${field}\n${r.stdout}`)
    assert.doesNotMatch(r.stdout, E007_HIT)
  }
})

test('E-007 不误报：借用形参（`&Arc<dyn Plugin>` 不可能是字段）', () => {
  const r = audit(withField('tree: &Arc<dyn Plugin>,'))
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E007_HIT)
})

test('E-007 不误报：多行函数签名的形参（行首在圆括号内，不是字段）', () => {
  const src = `use std::sync::Arc;

pub struct CompositePlugin {}

impl CompositePlugin {
    pub(crate) async fn broadcast_collect(
        plugin: Arc<dyn Plugin>,
        ctx: Arc<dyn PluginInvokeRequest>,
        who: &str,
    ) {
        let _ = (plugin, ctx, who);
    }
}
`
  const r = audit({ ...CLEAN, 'symbio/src/plugins/composite/composite.rs': src })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E007_HIT)
})

test('E-007 括号计数不被字符串带偏：含不平衡括号的字符串之后的真实字段仍命中', () => {
  const src = `const TIP: &str = "注意 (左括号不平衡";

pub struct FooPlugin {
    held: Option<Arc<dyn Plugin>>,
}
`
  const r = audit({ ...CLEAN, 'symbio/src/plugins/composite/composite.rs': src })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /\[ERROR\]\s+E-007\s+symbio\/src\/plugins\/composite\/composite\.rs:\d/)
})

test('E-007 不误报：不在 `plugins/` 之下的同类字段', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/symbio_core/registry.rs': 'pub struct Reg {\n    held: Arc<RwLock<Option<Arc<dyn Plugin>>>>,\n}\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E007_HIT)
})

test('E-007 豁免：带理由的 plugin-entry-allow 不再报', () => {
  const r = audit(
    withField('// plugin-entry-allow E-007: 本插件就是容器，按名持有子实例\n    held: Option<Arc<dyn Plugin>>,'),
  )
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E007_HIT)
})

test('E-007 豁免理由为空视为未豁免', () => {
  const r = audit(withField('// plugin-entry-allow E-007:\n    held: Option<Arc<dyn Plugin>>,'))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E007_HIT)
})

// ── 提取正确性：测试替身不得顶替生产实现 ──────────────────────────────
test('提取正确性：`*.test.rs` 里的假 route 不参与臂提取', () => {
  const r = audit({
    ...CLEAN,
    // walk 先深度再同级，`chat_loop/state.test.rs` 会排在 `plugin.rs` 之前；
    // 不过滤掉它，session 会被误判成「恒 NotFound、零路由臂」。
    'symbio/src/plugins/session/chat_loop/state.test.rs': `impl Plugin for FakePlugin {
    async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        if ctx.get(PATH).as_deref() != Some("hook/fire") {
            return Err(PluginError::NotFound(path));
        }
        Ok(PluginPayload::new(&1))
    }
}
`,
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /session\s+\[静态分派\]/)
  assert.match(r.stdout, /session\/chat\/send/)
})

// ── E-008：文档词表必须与代码常量逐字一致 ──────────────────────────────
//
// 真实事故的形状：`docs/design/vdfs.md` §3.2 的 status 行一直写 `error`，而代码
// 已把该词改名为 `failed`（理由见 `vdfs/words.rs::VDFS_STATUS_FAILED`）——两边
// 各说各话，没有任何测试因此变红；而人读文档写的代码会照 `error` 写。
const VOCAB_CORE = {
  'symbio/src/symbio_core/vdfs/words.rs':
    `pub const FOO_ONE: &str = "one";\npub const FOO_TWO: &str = "two";\n`,
}
const vocabDoc = (line) => ({ ...CLEAN, ...VOCAB_CORE, 'docs/design/vdfs.md': `${line}\n` })

test('E-008 命中：文档写着代码里已不存在的词', () => {
  const r = audit(vocabDoc('| `status` | `one` / `legacy` <!-- vocab:FOO_ --> |'))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /legacy/)
})

test('E-008 命中：代码新增了词而文档未列出', () => {
  const r = audit(vocabDoc('| `status` | `one` <!-- vocab:FOO_ --> |'))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /`two`/)
})

test('E-008 通过：两侧逐字一致', () => {
  const r = audit(vocabDoc('| `status` | `one` / `two` <!-- vocab:FOO_ --> |'))
  assert.equal(r.status, 0, r.stdout)
})

test('E-008 不误报：反引号里的大写常量名不是取值', () => {
  const r = audit(vocabDoc('| `status` | `one` / `two`（空串见 `FOO_NONE`）<!-- vocab:FOO_ --> |'))
  assert.equal(r.status, 0, r.stdout)
})

test('E-008 不猜：没有 `<!-- vocab:… -->` 标记的行不判', () => {
  const r = audit(vocabDoc('| `status` | `one` / `legacy` |'))
  assert.equal(r.status, 0, r.stdout)
})

test('E-008 命中：标记的前缀在代码里没有同名常量（前缀写错）', () => {
  const r = audit(vocabDoc('| `status` | `one` <!-- vocab:BAR_ --> |'))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /BAR_/)
})

// ── E-009：插件不得直接 use 兄弟插件模块 ────────────────────────────────
//
// 真实形态：`plugins/mod.rs` 的架构原则说「插件之间互相不可见」，但 Rust 的私有
// 可见性包含"定义模块的后代" ⇒ `plugins::mcp` 能路径到 `plugins::web`。当前代码为 0，
// 但没有任何守卫；这条规则把它钉住。
const E009_HIT = /\[ERROR\]\s+E-009\s+symbio\/src\/plugins\/mcp\/caller\.rs:\d/

test('E-009 命中：`plugins/mcp` 里 `use crate::plugins::web::…`', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs': 'use crate::plugins::web::something;\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E009_HIT)
})

test('E-009 不误报：引用自己的插件模块', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs': 'use crate::plugins::mcp::schemas::Foo;\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E009_HIT)
})

test('E-009 不误报：文档链接（注释里的 `crate::plugins::web`）', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      '//! 与 [`web`](crate::plugins::web) 完全一致\npub fn f() {}\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E009_HIT)
})

test('E-009 不误报：不在 `plugins/` 之下的同类引用（如 `cli/`）', () => {
  const r = audit({
    ...CLEAN,
    'cli/src/client.rs': 'use crate::plugins::web::something;\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E009_HIT)
})

test('E-009 豁免：带理由的 plugin-entry-allow 不再报', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      '// plugin-entry-allow E-009: 本插件是刻意的容器替身，需引用兄弟类型\nuse crate::plugins::web::something;\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E009_HIT)
})

test('E-009 豁免理由为空视为未豁免', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      '// plugin-entry-allow E-009:\nuse crate::plugins::web::something;\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E009_HIT)
})

// ── E-010：消费方不得深引 `symbio_core::<域>::` ─────────────────────────
//
// 真实形态：`symbio_core/README.md` §1.4 规定「根平铺导出是唯一出口」，但这条规则
// 此前**没有任何守卫**，于是烂到 14 处。深引的代价不是「不好看」——根导出是唯一的
// 公开面，符号从根移除后深引点照样编译通过，公开面于是变成两套。
const E010_HIT = /\[ERROR\]\s+E-010\s+\S+:\d/

test('E-010 命中：插件里 `crate::symbio_core::<域>::…`', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs': 'use crate::symbio_core::vdfs::VdfsProvider;\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E010_HIT)
})

test('E-010 命中：跨 crate 的 `symbio::symbio_core::<域>::…`（cli）', () => {
  const r = audit({
    ...CLEAN,
    'cli/src/client.rs': 'use symbio::symbio_core::event_bus::EventBus;\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E010_HIT)
})

test('E-010 命中：壳侧 `tauri/src-tauri` 也管（整棵插件树编译进壳）', () => {
  const r = audit({
    ...CLEAN,
    'tauri/src-tauri/src/commands.rs': 'use symbio::symbio_core::plugin::Plugin;\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, E010_HIT)
})

test('E-010 不误报：`schemas::` 是 §1.4 明文允许的唯一深引', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      'use crate::symbio_core::schemas::session::chat_message::ChatMessage;\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E010_HIT)
})

test('E-010 不误报：文档链接（注释里的深引路径）', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      '//! 见 [`VdfsProvider`](crate::symbio_core::vdfs::VdfsProvider)。\npub fn f() {}\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E010_HIT)
})

test('E-010 不误报：根平铺写法（`symbio_core::<符号>`）', () => {
  const r = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      'use crate::symbio_core::{VdfsProvider, VdfsNode};\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.doesNotMatch(r.stdout, E010_HIT)
})

test('E-010 豁免：带理由的 plugin-entry-allow 不再报；空理由仍报', () => {
  const withReason = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      '// plugin-entry-allow E-010: 本文件要按子模块路径重导出\nuse crate::symbio_core::vdfs::VdfsProvider;\n',
  })
  assert.equal(withReason.status, 0, withReason.stdout)
  assert.doesNotMatch(withReason.stdout, E010_HIT)

  const emptyReason = audit({
    ...CLEAN,
    'symbio/src/plugins/mcp/caller.rs':
      '// plugin-entry-allow E-010:\nuse crate::symbio_core::vdfs::VdfsProvider;\n',
  })
  assert.equal(emptyReason.status, 1, emptyReason.stdout)
  assert.match(emptyReason.stdout, E010_HIT)
})
