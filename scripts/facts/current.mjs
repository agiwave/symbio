#!/usr/bin/env node
/**
 * 事实 · `docs/004-ref/006-当前事实表.md` —— 读侧结构事实表
 *
 * 动机（真实 agent 会话自评估驱动）：审查本仓库的 agent 反复反馈"现在是什么"
 * 这张事实表不存在——只有「写侧论证」（ADR 全在讲为什么）与"会漂移的手抄清单"
 * （README / 003-ROUTES.md 都曾漂移）。于是核对一条事实要 1-3 轮往返，核对成本高到
 * agent 选择"不核对、用自信语气包装未验证结论"。
 *
 * 本模块**只从代码抽取**（提取，不手写）：每一行都能追到源文件，
 * 因此它不会像手抄清单那样漂移。它只回答"现在是什么"（结构）；
 * "为什么"仍看 docs/003-design/（每条 ADR 的正文 + 状态；索引见 docs/003-design/README.md）。
 *
 * ## 它怎么被调用
 *
 * **不要直接跑它**——`scripts/gen-facts.mjs` 遍历 `scripts/facts/` 逐个调用本文件
 * 导出的描述符。直接跑与 runner 走的是同一份 `render()`，但绕过了目录遍历与
 * 契约校验（那份校验才是「加了文件忘了接线」的唯一检出点）。
 *
 * 口径（诚实优先于完整）：
 *   - 静态可提取的一律提取；提取不到的**标注为动态**，绝不臆造
 *     （如 `mcp` 运行期桥接的工具、`local/shell` 按操作系统取名的工具）。
 *   - 路由：§2.3 是**静态路由地址与一句话职责的唯一表**（代码投影）。契约
 *     （调用方 / 返回类型 / 备注）只有人知道，住 RE-003 的契约表——以地址为键，
 *     不在这里也不在别处成第二张地址表；「键必须是真路由」由 facts.test.mjs 判
 *     （多一个键判红，`_root` 病史 + 假键两个反向用例）。
 *     走过的弯路记在这里防再犯：曾让 RE-003 用 `view` 把本表与契约表**合并重渲**
 *     （一个 owner、两个投影）——view 已整体取消，投影重渲是把重复换个形态留下，
 *     生成的重复也是重复。
 *   - 不猜 VDFS 挂载点的资源存储选型（`vdfs_service` 的 Single/Dir/Memory 是
 *     实现细节，见 `docs/003-design/009-VDFS规范.md` §11）。
 */

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { scopeRow as sharedScopeRow } from "../line-count.mjs";
import { stripComments } from "../rust-scan.mjs";
import { capabilityFacts, UNKNOWN, vdfsAddrRoot } from "./_capability.mjs";
import { createRefLink } from "./_ref-link.mjs";
// 路由提取只有一份实现（`route-facts.mjs`），与前端常量生成器共用——
// 「什么算一条路由」这条判据若有两遍，它们会在没人注意时分叉。
import { pluginSources, extractRouteArms, vdfsOps } from "../route-facts.mjs";

// 本文件住 `scripts/facts/`，故仓根要上溯两级（原来在 `scripts/` 下只需一级）。
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const PLUGINS_DIR = path.join(ROOT, "crates");
const PLUGIN_CRATE_PREFIX = "symbio-plugin-";
// 生成物里的编号引用：展开形态与 `gen-doc-refs` 逐字一致（`_lib.mjs` 的病历：
// 9f49d454 只有用法没有定义，6 处字面量静默上盘——执行方是 facts.test.mjs）。
// fromDir = 本文件的产物目录（`docs/ref`）。
const refLink = createRefLink(ROOT, "docs/ref");
// 插件工厂 id 的 owner 是装配方（`crates/symbio-deploy/src/lib.rs`，由每个插件目录的
// `PLUGIN.yml` 生成）——它们描述的是「哪个插件」，不是「哪个键」，也不属于内核机制
// （见 `assembly.rs` 模块文档）。
const IDS_FILE = path.join(ROOT, "crates", "symbio-deploy", "src", "lib.rs");
/**
 * 输出位置：**`docs/004-ref/` 下的一份编号参考文档**。
 *
 * 为什么从 `docs/004-ref/006-当前事实表.md` 搬进来：原先它是个**登记体系外**的特例——免登记
 * （`SCAN_SKIP_FILES`）、没有编号、于是别人引用它只能手写 `../CURRENT.md#…`。
 * 那条手写地址是「靠眼睛同步」的活：本仓曾为它专门加了一条判据盯着节次漂移，
 * 而那条判据本来不该存在——**编号引用由 `gen-doc-refs` 重算地址**，锚点之外的一切
 * 都不该是人手维护的。搬进 `ref/` 并给编号后，它和其它 5 篇 `RE-` 一样：
 * 进 `INDEX`、被编号引用（`RE-006-当前事实表`）、判据一视同仁。
 *
 * `id` 与文件名逐字一致（RG-003），`title` 与 H1 逐字一致（RG-021），
 * `created` 是**本仓首次生成它的日期**而不是「本次运行的时间」——
 * 时间戳逐次变化的字段会让 `--check` 永远判漂移（见 `_lib.mjs` 的 stripStamp）。
 */
const OUT_REL = "docs/004-ref/006-当前事实表.md";
const ENTRY_ID = "RE-006-当前事实表";
const ENTRY_TITLE = "Symbio 当前事实表（自动生成）";
const ENTRY_CREATED = "2026-09-25";

/** 收集文件内的 `const X: &str = "…"` 常量（用于解析 `name: CONST.to_string()`） */
function parseConsts(txt, map) {
  for (const mm of txt.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    map.set(mm[1], mm[2]);
  }
  return map;
}

/** 装配方的 id 表：`pub const PLUGIN_ID_LOCAL: &str = "local";` → Map 常量名 → 值 */
function parseIds(txt) {
  const m = new Map();
  for (const mm of txt.matchAll(/pub const (PLUGIN_[A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    m.set(mm[1], mm[2]);
  }
  return m;
}

/** 解析 `PluginMeta::new(X` / `CapabilityRow::mount(X` 的第一个实参 */
function resolveArg(raw, ids) {
  const t = raw.trim();
  if (t.startsWith('"')) return t.slice(1, t.lastIndexOf('"'));
  if (/^PLUGIN_[A-Z_]+$/.test(t)) return ids.get(t) ?? `?${t}`;
  return "（运行期动态）";
}

/** 从 CapabilityMeta 字面量 / 工具构造 helper 里提取 LLM 可见工具短名 */
function extractToolNames(t, consts) {
  const out = new Set();
  for (const mm of t.matchAll(/CapabilityMeta\s*\{[\s\S]{0,400}?\bname:\s*"([a-z][a-z0-9_]*)"/g)) {
    out.add(mm[1]);
  }
  // `name: CONST_NAME.to_string()` —— 名字由常量给出（如 subagent 的 `NAME`）
  for (const mm of t.matchAll(
    /CapabilityMeta\s*\{[\s\S]{0,400}?\bname:\s*([A-Za-z_][A-Za-z0-9_:]*)\s*\.to_string\(\)/g
  )) {
    const key = mm[1].split("::").pop();
    const v = consts.get(key);
    if (v && /^[a-z][a-z0-9_]{2,}$/.test(v)) out.add(v);
  }
  for (const mm of t.matchAll(/\btool\(\s*"([a-z][a-z0-9_]*)"/g)) {
    out.add(mm[1]);
  }
  // `ToolSpec { name: "vdfs_*" }` 表项（`plugins/vdfs/tools/spec.rs` 的同形工具表）——
  // 九个 vdfs 工具走这张 static 表，不是 `CapabilityMeta` 字面量，漏掉它 §2 会
  // 少七行（2026-10-14 补：e2e LLM 调用契约钉运行时工具面时发现文档与实测不符）。
  // `name` 是 `ToolSpec` 的首字段，故贴名匹配不会误伤别处的 `name:`。
  for (const mm of t.matchAll(/ToolSpec\s*\{\s*name:\s*"([a-z][a-z0-9_]*)"/g)) {
    out.add(mm[1]);
  }
  return [...out].sort();
}

/** 核心 trait 白名单：只统计真正定义插件形态的 trait */
const CORE_TRAITS = ["Plugin", "VdfsProvider", "Capability", "ModelProvider", "ConfigurableVisitor"];

/** 单个插件的静态事实 */
function analyzePlugin(crateDirName, ids) {
  const dirName = crateDirName.startsWith(PLUGIN_CRATE_PREFIX)
    ? crateDirName.slice(PLUGIN_CRATE_PREFIX.length)
    : crateDirName;
  const dir = path.join(PLUGINS_DIR, crateDirName);
  const contents = pluginSources(dir);
  const consts = new Map();
  for (const t of contents) parseConsts(t, consts);

  // `PluginMeta::new` 的**首参**（`id`）——注意不是 `name`（第二参）。
  // 它只用于本表的「注册名」列：**不参与路由**，路由前缀取目录名（见 extractRouteArms）。
  let metaId = null;
  const mounts = new Set();
  const tools = new Set();
  const traits = new Set();
  const routes = new Set();
  let hasConfig = false;

  for (const t of contents) {
    if (!metaId) {
      const mm = t.match(/PluginMeta::new\(([^,\n]+)/);
      if (mm) metaId = resolveArg(mm[1], ids);
    }
    // 挂载点 = 「插件交得出 VDFS 视图」这一件事：`get_vfs_provider` 返回 `Some`。
    // 运行时容器用同一判据派生挂载行（见 `composite.rs::traverse`），目录名 = 实例名；
    // 插件自己不再注册挂载行。`composite` 除外：它是 `<根>` 的服务者，不是子目录。
    if (dirName !== "composite" && /fn get_vfs_provider[\s\S]{0,200}?Some\(/.test(t)) {
      mounts.add(dirName);
    }
    if (t.includes("capability_announce_configurable(")) hasConfig = true;
    for (const n of extractToolNames(t, consts)) tools.add(n);
    for (const mm of t.matchAll(
      /impl\s+(?:<[^>]*>\s*)?([A-Z][A-Za-z0-9_]*)(?:<[^>]*>)?\s+for\s+/g
    )) {
      if (CORE_TRAITS.includes(mm[1])) traits.add(mm[1]);
    }
    for (const arm of extractRouteArms(t, dirName)) routes.add(arm);
  }

  return {
    dirName,
    metaId,
    mounts: [...mounts].sort(),
    tools: [...tools].sort(),
    traits: [...traits].sort(),
    routes: [...routes].sort(),
    hasConfig,
    hasReadme: existsSync(path.join(dir, "README.md")),
  };
}

function scopeRows() {
  return [
    sharedScopeRow(ROOT, "crates", [".rs"]),
    sharedScopeRow(ROOT, "crates/symbio-kernel/src", [".rs"]),
    sharedScopeRow(ROOT, "crates/symbio-cli/src", [".rs"]),
    sharedScopeRow(ROOT, "tauri/src-tauri/src", [".rs"]),
    sharedScopeRow(ROOT, "tauri/src", [".ts", ".vue"]),
  ];
}

/** `generate_handler![…]` 里**实际注册**的 command（先剥注释，被注释掉的不算接缝） */
function tauriCommands() {
  const file = path.join(ROOT, "tauri", "src-tauri", "src", "main.rs");
  let txt;
  try {
    txt = stripComments(readFileSync(file, "utf8"));
  } catch {
    return [];
  }
  const m = txt.match(/generate_handler!\s*\[([\s\S]*?)\]/);
  if (!m) return [];
  return [...m[1].matchAll(/(?:commands|crate)::([a-z0-9_]+)|^\s*([a-z0-9_]+)\s*,/gm)]
    .map((g) => g[1] ?? g[2])
    .filter(Boolean);
}

/** `#[tauri::command]` 声明总数（与 generate_handler 注册数对比——差值才是「未注册历史函数」，不许硬编码） */
function tauriCommandDeclared() {
  const dir = path.join(ROOT, "tauri", "src-tauri", "src");
  let n = 0;
  const walk = (d) => {
    for (const ent of readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, ent.name);
      if (ent.isDirectory()) walk(p);
      else if (ent.name.endsWith(".rs"))
        n += (readFileSync(p, "utf8").match(/#\[tauri::command\]/g) ?? []).length;
    }
  };
  try {
    walk(dir);
  } catch {
    /* 目录缺失按 0 计，由差值分支自然处理 */
  }
  return n;
}

/** 前端 router：route 条数与真实组件数（redirect 不算组件——视图收敛程度是可数的事实） */
function tauriViews() {
  const file = path.join(ROOT, "tauri", "src", "router", "index.ts");
  let txt;
  try {
    txt = readFileSync(file, "utf8");
  } catch {
    return { routes: 0, components: 0, components_list: [] };
  }
  const routes = [...txt.matchAll(/\{\s*(?:path|redirect)/g)].length;
  const comps = [...new Set([...txt.matchAll(/component:\s*([A-Za-z]\w*)/g)].map((g) => g[1]))];
  return { routes, components: comps.length, components_list: comps };
}

/**
 * CLI 面：进程选项 + REPL 内置命令。
 *
 * 权威来源是 `crates/symbio-cli/src/args.rs` 的 `HELP` 常量（无子命令树、不依赖 clap）。
 * 之所以用提取而非手写清单：手写时我把「交互模式内置命令」当成了不存在的
 * 「子命令」，还漏掉了 `--heartbeat`——事实表里的清单一旦手写就会腐烂。
 */
function cliSurface() {
  const file = path.join(ROOT, "cli", "src", "args.rs");
  let txt;
  try {
    txt = readFileSync(file, "utf8");
  } catch {
    return { longs: [], shorts: [], repl: [] };
  }
  const m = txt.match(/const HELP: &str = "\s*([\s\S]*?)\n"/);
  const help = m ? m[1] : "";
  const section = (from, to) => {
    const a = help.indexOf(from);
    if (a < 0) return "";
    const rest = help.slice(a + from.length);
    const b = to ? rest.indexOf(to) : -1;
    return b < 0 ? rest : rest.slice(0, b);
  };
  const optBlock = section("选项:", "交互模式内置命令:");
  const replBlock = section("交互模式内置命令:", "输出约定:");
  return {
    longs: [...new Set([...optBlock.matchAll(/--[a-z][\w-]*/g)].map((g) => g[0]))],
    shorts: [...new Set([...optBlock.matchAll(/(?:^|\s)-([A-Za-z])(?=[\s,])/g)].map((g) => `-${g[1]}`))],
    repl: [...new Set([...replBlock.matchAll(/(?:^|\s)(\/[\w-]+)/g)].map((g) => g[1]))],
  };
}

/**
 * Gateway 对外端点：从 `server.rs` 的分派代码提取，而不是从 README 抄。
 *
 * 端点由 `req.path.starts_with("…")` 决定，所以正则扫代码就是权威；插件 README
 * 只指向 003-ROUTES.md、不再自列端点（早年 README 写 `/api/route`、`/api/ws` 而代码
 * 是 `/api/v1/invoke`、`/api/v1/health`，是文档腐烂的活案例）。
 */
function gatewayEndpoints() {
  const file = path.join(ROOT, "crates", "symbio-plugin-gateway", "src", "server.rs");
  let txt;
  try {
    txt = stripComments(readFileSync(file, "utf8"));
  } catch {
    return { http: [], ws: false };
  }
  const http = [
    ...new Set(
      [...txt.matchAll(/req\.method == "(\w+)" && req\.path\.starts_with\("([^"]+)"\)/g)].map(
        (g) => `${g[1]} ${g[2]}`
      )
    ),
  ];
  return { http, ws: /fn\s+ws_handshake/.test(txt) };
}

// ================= 渲染 =================

// ⚠️ 本文件**不含任何人写的表格**。
//
// 这条纪律是 2026-10-14 补的：这里曾有一张 `TOOL_NOTES`（20 条「工具 → 一句备注」），
// 它是整份事实表里唯一的手写内容，与 §2.1 的 `CapabilityMeta.description`（代码提取）
// 逐条讲同一件事——于是它既冗余又**会漂移**：代码改了描述，备注不会跟着改，而它看起来
// 和别的列一样是「从代码来的」。那份文件的报头写着「从代码提取生成，请勿手改」，一张
// 手写表与这句话直接矛盾。
//
// 今天每一条工具语义只有一个出处（`facts/_capability.mjs` 从 `CapabilityMeta` 读），
// 没有任何一句话需要人来同步。**新增工具不需要动这个文件**——这正是要买断的东西。

/** 静态抽取拿不到的工具来源：明确标注，绝不臆造 */
const DYNAMIC_TOOLS = {
  local:
    "`shell` 族工具名按操作系统取（Windows = `cmd`），静态抽取只见占位名 → 以运行时 `traverse(\"available_tools\")` 为准",
  mcp: "桥接工具在运行期按已连接 server 动态生成，无法静态枚举",
};

/** 路由按运行期规则分发、无法静态枚举的插件：标注而不是留空 */
const DYNAMIC_ROUTES = {
  local: "`local/<工具短名>`——按已注册工具名分发（与 §2.1 的工具清单同一份集合）",
  // 操作数从协议源码动态推导（VDFS_OPS 增删时不再漂移）
  vdfs: `\`vdfs/<操作>\`——按 \`VDFS_OPS\` 校验后分发（见 §2.2，${
    vdfsOps(ROOT).length
  } 个操作）`,
  composite: "容器：按配置挂载的子插件名分发，运行期动态",
  agent: "已无自有路由（一律 `NotFound` 并指引到 `<根>/agent`）",
  model: "已无自有路由（模型往返经 `ModelProvider::round_trip` 进程内调用）",
};

const fmtList = (items) => (items.length ? items.map((x) => `\`${x}\``).join(" · ") : "—");
const fmtMounts = (items) =>
  items.length
    ? items.map((m) => (m.startsWith("（") ? m : `<根>/${m}`)).join(" · ")
    : "—";

function headSha() {
  try {
    return execSync("git rev-parse --short HEAD", { cwd: ROOT, encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

function render() {
  const ids = parseIds(readFileSync(IDS_FILE, "utf8"));
  const pluginDirs = readdirSync(PLUGINS_DIR, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith(PLUGIN_CRATE_PREFIX))
    .filter((e) => e.name !== "symbio-plugin-api")
    .map((e) => e.name)
    .sort();
  const plugins = pluginDirs.map((d) => analyzePlugin(d, ids));
  // 静态可提取的地址全集（§3.1 报条数用；与前端生成物同一份提取口径）
  const staticRoutes = [...new Set(plugins.flatMap((p) => p.routes))].sort();

  const L = [];
  // front-matter **整体由生成器发出**：一份登记条目该有哪些字段，只有渲染它的人知道。
  // `lists` 尤其不能让人手写——那等于让「代码投影」里混进人写的内容，正是
  // `TOOL_NOTES` 的病根。kind / id / title / status / created 是登记体系的硬要求
  // （RG-003 / RG-021 / RG-004 / RG-005），少一条这条文档就掉出登记之外。
  L.push("---");
  L.push("kind: ref");
  L.push(`id: ${ENTRY_ID}`);
  L.push(`title: ${ENTRY_TITLE}`);
  L.push("status: active");
  L.push(`created: ${ENTRY_CREATED}`);
  L.push("lists: [routes=owner, tools=owner, vdfs-ops=owner, cli-options=owner]");
  L.push("---");
  L.push("");
  L.push(`# ${ENTRY_TITLE}`);
  L.push("");
  L.push("> ⚠️ **本文件由 `scripts/facts/current.mjs` 从代码提取生成，请勿手改。**");
  L.push("> 改了代码不用手动重跑——门禁会**自动重新生成并暂存**（`scripts/gen-facts.mjs`，见 `scripts/gate.d/60-facts.mjs`）。");
  L.push(">");
  L.push("> 本表回答「**现在是什么**」（结构；代码投影，不会漂移）；");
  L.push(`> 「**为什么这样设计**」看该主题设计文档的「关键决策」节（每条决策一个 \`### D-NNN\` 条目；总览见 ${refLink('DE-000-设计总览', '003-design/README.md')}）；`);
  L.push(`> 路由的**契约**（调用方 / 返回类型 / 备注）看 ${refLink('RE-003-ROUTES', '003-ROUTES.md')}（静态骨架与本表同源，行集相等有判据）。`);
  L.push("");
  L.push("<!-- AUTO-GENERATED by scripts/facts/current.mjs — do not edit by hand -->");
  L.push("");

  // ── §1 插件总表 ─────────────────────────────────────────────────────
  L.push("## 1. 插件总表（`crates/plugins/`）");
  L.push("");
  L.push(
    "| 插件目录 | 注册名 | VDFS 挂载点 | 自有路由（静态可提取） | 实现的核心 trait | 配置文件 | 模块 README |"
  );
  L.push("|---|---|---|---|---|---|---|");
  for (const p of plugins) {
    // 静态路由**不在此逐条列**——同一批字符串在 §2.3 已有完整表，逐条写两遍会让读者
    // 不知道该信哪一格。本格只答「这个插件对外开了几条」这个**插件视角**的问题，
    // 逐条职责去 §2.3（路由视角）。两格各答一问，不是同一问的两次回答。
    // 动态路由必须留在本格：它们没有可静态枚举的臂，§2.3 装不下。
    const routeCell = p.routes.length
      ? `${p.routes.length} 条静态（见 §2.3）`
      : DYNAMIC_ROUTES[p.dirName]
        ? `（动态）${DYNAMIC_ROUTES[p.dirName]}`
        : "—";
    L.push(
      `| \`${p.dirName}\` | \`${p.metaId ?? "—"}\` | ${fmtMounts(p.mounts)} | ${routeCell} | ${fmtList(
        p.traits
      )} | ${p.hasConfig ? "✓" : "—"} | ${p.hasReadme ? "✓" : "**缺**"} |`
    );
  }
  L.push("");
  L.push("> 读表须知：");
  L.push("> - **挂载点** = 该插件目录名（容器实例表的挂载名，`目录名 = 实例名`）；");
  L.push(">   判据只有一条：`Plugin::get_vfs_provider` 返回 `Some`（系统 / 前端链路直接查询，");
  L.push(">   容器聚合）。**LLM 链路**由容器在收集广播里按同一判据派生挂载行");
  L.push(">   （`put(CapabilityRow::mount(实例名, provider))`），插件不自行注册——两条通道因此");
  L.push(">   不可能各说各话。");
  L.push(">   容器（`composite`）自身即 `<根>` 的服务者，其挂载点是运行期动态。");
  L.push(">   资源存储的**选型**（`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs`）是实现细节，不在本表出现。");
  L.push("> - **自有路由** = `async fn route()` 体内 `match` 臂的字符串（臂是**相对路径**，");
  L.push(">   容器已剥掉首段，故地址前缀是**插件目录名**——容器按目录名建实例表并按它分发；");
  L.push(">   `PluginMeta` 首参不参与路由——它只是**出厂 id**，");
  L.push(">   插件**身份**取自 `PLUGIN.yml`（见 [D-032](../003-design/002-架构总览.md#d-032-插件身份归-pluginymlpluginmeta-从元信息降为出厂自述 'DE-002-架构总览')）。");
  L.push(">   本格只报**条数**（插件视角）；逐条地址与职责见 §2.3（路由视角）。");
  L.push(">   标「（动态）」的是按运行期规则分发、无法静态枚举的——它们只在本格出现。");
  L.push(">   路由契约（调用方 / 返回类型）见 " + refLink('RE-003-ROUTES', '003-ROUTES.md') + "。");
  L.push("> - **配置文件** = 该插件调用过 `capability_announce_configurable`（配置就是 `<根>/<挂载点>/PLUGIN.yml`，");
  L.push(">   读写走 `vdfs/read` / `vdfs/write`，**没有配置专用路由**）。");
  L.push("");

  // ── §2 能力矩阵（原来排在 §6；它覆盖了原 §2 的工具清单，故提上来占位）────
  //
  // **为什么它在第 2 节而不是末尾**：它是这份表里信息量最大的一节（每条能力 + 它
  // 的用户可见语义），排在末尾只因为它是后加的。更实际的理由是：它与「覆盖面」两节
  // 讲的是同一批对象，必须挨着，否则读者会把「§2 的工具清单」和「§6.1 的工具语义」
  // 当成两张表——那正是本节被合并前的重复形态。
  L.push(capabilityMatrixSection(ROOT, plugins));

  // ── §3 协议速查 ─────────────────────────────────────────────────────
  L.push("## 3. 协议速查");
  L.push("");
  L.push("### 3.1 前端 / 宿主路由");
  L.push("");
  L.push("```");
  L.push("{container}/{plugin}/{action}     例：worker/session/stats（worker 可省略）");
  L.push("```");
  L.push("");
  // 条数只在 §2.3 报一次（同一数字写两遍，读的人会以为是两个口径）。
  L.push(`- 静态可提取地址共 **${staticRoutes.length}** 条，逐条与各自职责见 §2.3；`);
  L.push(
    `- **前端侧常量**：\`tauri/src/constants/routes.gen.ts\`——同 ${staticRoutes.length} 条，` +
      "由后端 `route()` 的 `match` 臂**生成**（`scripts/facts/routes-ts.mjs`），不是手写登记处；" +
      "前端在别处写死同一条地址由 `mechanism-audit` 的 M-008 判红。",
  );
  L.push(
    "- 路由**契约**（调用方 / 返回类型 / 退役索引）不属本表，归 " + refLink('RE-003-ROUTES', '003-ROUTES.md') + "。",
  );
  L.push("");
  // ⚠️ 这里**不列** VDFS 操作名与 LLM 工具名：两样都已在 §2.2 / §2.1 逐条列出。
  // 本节只答一句两条链路**不重合**的话——那是列表本身给不出的信息。
  L.push("### 3.2 VDFS 的两条链路不是一一对应");
  L.push("");
  L.push(
    "同 `plugins/vdfs/protocol.rs::VDFS_OPS` 的操作，前端链路与 LLM 工具链路**各有各的子集**：" +
      "逐条操作见 §2.2，工具短名见 §2.1（`vdfs_*` 那九条）。" +
      "`watch` / `unwatch` / `action` **不经工具暴露**，故工具面是操作面的真子集。",
  );
  L.push("");

  // ── §4 存储层事实 ───────────────────────────────────────────────────
  L.push("## 4. 存储层事实（防「多存储后端」误读）");
  L.push("");
  L.push("| 数据 | 位置 | 后端 |");
  L.push("|---|---|---|");
  L.push(
    "| 资源型插件条目（agent / model / mcp / skill / plugin_manager …） | `<homedir>/<类别>/<id>/<主文件>` | `providers/vdfs_service/` 三型：`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs` |"
  );
  L.push(
    "| 会话与其消息 | `<homedir>/session/<id>/{session.json,messages.json}` | **单一具体类型** `SessionStore`（持久=磁盘布局 / 临时=进程内驻留）——**没有后端选型维度**，要换存储就是换类型 |"
  );
          const vdfsRoot = "<vdfs_root>"; // grep-audit-allow S-010: CURRENT.md 约定用占位，不写字面量
  // 真实值来自 plugins/vdfs/fs.rs::VDFS_ADDR_ROOT（运行期抽取，防漂移）；
  // 输出仅做占位，不直接写字面量。
  if (!vdfsAddrRoot()) {
    throw new Error("VDFS_ADDR_ROOT 为空，CURRENT.md §4 回写缺陷");
  }
  L.push(
    `| Agent 目录 | \`agent/<id>\`（工作区级 + 全局级双层） | \`AgentDirStore\` 自管，不经 \`vdfs_service\`；虚拟视图以 \`${vdfsRoot}/agent/<id>\` 进入 |`
  );
  // ⚠️ 路径**不带 `plugins/` 层**：那一层早已废除（`symbio_core/plugin/dir.rs`
  // 顶部写明了理由与自举环），系统根下就是「一个插件一个目录」的扁平结构。
  // 这里曾长期写着 `<homedir>/plugins/<插件>/PLUGIN.yml`，与代码和 001-CONFIGURATION.md
  // 都不符——而本表自称「权威事实」，错了会被逐字抄进别处。
  L.push(
    "| 插件配置（含会话配置） | `<homedir>/<插件>/PLUGIN.yml`（系统级在 `<homedir>/PLUGIN.yml`） | `PluginConfigFile` 自读写，**无第二种后端、无第二条配置协议** |"
  );
  L.push("");

  // ── §5 规模与宿主接缝（可验证的量化事实）──────────────────────────
  L.push("## 5. 规模与宿主接缝");
  L.push("");
  L.push("### 5.1 代码规模（不含 `target/` `vendor/` `node_modules/` `dist/`，实现与测试分列）");
  L.push("");
  L.push("| 范围 | 实现 | 测试 |");
  L.push("|---|---|---|");
  for (const s of scopeRows()) {
    L.push(
      `| \`${s.dir}\` | ${s.implFiles} 文件 / ${s.implLines} 行 | ${s.testFiles} 文件 / ${s.testLines} 行 |`
    );
  }
  L.push("");
  L.push("### 5.2 宿主接缝（前端到底有多大）");
  L.push("");
  const ipc = tauriCommands();
  const unregisteredN = tauriCommandDeclared() - ipc.length;
  L.push(
    `- **Tauri IPC**：注册 ${ipc.length} 个 command —— ${fmtList(ipc)}（` +
      "`tauri/src-tauri/src/main.rs::generate_handler!`" +
      (unregisteredN > 0
        ? `；另有 ${unregisteredN} 个已声明未注册的 \`#[tauri::command]\`，不计入接缝`
        : "") +
      "）"
  );
  const views = tauriViews();
  L.push(
    `- **前端路由**：${views.routes} 条 route，其中真实组件 ${views.components} 个（${fmtList(
      views.components_list
    )}）；其余为旧地址 ` +
      "`redirect`。即「一台控件承载全部资源类型」在代码里可数。"
  );
  const gw = gatewayEndpoints();
  L.push(
    `- **Gateway 端点**：${fmtList(gw.http)}${gw.ws ? " + WS 升级（任意 path，首帧 = `PluginMessageWire`）" : ""}` +
      "（提取自 `gateway/server.rs` 的 `req.path.starts_with`）"
  );
  const cli = cliSurface();
  L.push("<!-- list:cli-options owner -->");
  L.push("");
  L.push(
    `- **CLI 面**：进程选项 ${cli.longs.length} 个长 + ${cli.shorts.length} 个短（${fmtList(
      cli.longs
    )}）；交互模式内置命令 ${cli.repl.length} 个（${fmtList(
      cli.repl
    )}）。无子命令树、不依赖 clap，权威来源是 \`crates/symbio-cli/src/args.rs\` 的 \`HELP\`。` +
      ' 本清单的 owner 是本行：它从 `crates/symbio-cli/src/args.rs` 的 `HELP` 提取，改代码即变。' +
      '`crates/symbio-cli/README.md` 与 `crates/symbio-cli/docs/004-usage.md` 是它的 `view`（只指路，不抄表）。'
  );
  L.push("");

  // ── 生成信息 ──────────────────────────────────────────────────────
  L.push("---");
  L.push("");
  const stamp = new Date().toISOString().slice(0, 19).replace("T", " ");
  L.push(`> 生成时间：${stamp} UTC · 源：\`git rev-parse HEAD\` = \`${headSha()}\``);
  L.push("");
  return L.join("\n");
}

/**
 * 表格单元格转义：**`|` 必须转义**。
 *
 * 不是洁癖——`vdfs_list` 的描述里就写着 `type（directory|file）`，一个裸竖线会被
 * Markdown 读成新的一列，于是那一行的其余列整体错位，而生成器**看不见**这件事
 * （它只管吐出文本）。语义取自代码 ⇒ 代码里有什么字符都会进表格，所以转义得
 * 在**渲染这一层**做，不能指望源码里没有竖线。
 */
const cell = (s) => String(s ?? "").replace(/\|/g, "\\|").replace(/\s*\n\s*/g, " ").trim();

/**
 * 能力文本里的**根名字面量**替换成 `<vdfs_root>` 占位。
 *
 * 为什么必须遮：仓级守卫 S-010 规定 VDFS 挂载根的字面量只归 `plugins/vdfs/fs.rs`
 * 一处，消费方一律取运行期值。工具描述是**喂给模型的文案**，它在代码里提到那个目录
 * 是合理的；可本表一旦把它原样吐进文档，就等于给根名造了第二处出现——而 §4 的存储
 * 布局表早就用 `grep-audit-allow S-010` 的同一套占位处理过这件事。
 *
 * 取值来自运行期抽取（`vdfsAddrRoot()` 读 `plugins/vdfs/fs.rs`），因此这里不硬编码
 * 任何字面量，根名改了这里自动跟着改。
 */
function maskRoot(text) {
  const root = vdfsAddrRoot();
  if (!root) return String(text ?? "");
  return String(text ?? "").split(root).join("<vdfs_root>");
}

/**
 * §6 能力矩阵：每条能力 + 它的**用户可见语义**，全部取自代码。
 *
 * 本节（§2）是**能力面的唯一 owner**——覆盖面在 §1，语义在这里，两者讲同一批对象但不重复列。
 *
 * 语义的三处出处（提取实现见 `facts/_capability.mjs`）：
 * | 面 | 出处 |
 * |---|---|
 * | LLM 工具 | `CapabilityMeta.description`（模型与前端看到的原文）＋ `category` / `risk` |
 * | VDFS 操作 | `VDFS_*` 常量正上方的文档注释 |
 * | 路由 | 插件 `route()` 里那条 `match` 臂正上方的注释 |
 *
 * **缺的一律写 `—`，绝不替它编一句**（同一条纪律的反面教材曾是这个文件里那张手写
 * `TOOL_NOTES` 表——它与本节的描述列逐条讲同一件事，既冗余又不会跟着代码走）。
 */
function capabilityMatrixSection(root, plugins) {
  const cap = capabilityFacts(root)
  // 工具 → 贡献插件：复用插件表（同一份提取，两处不各判一次）
  const owner = new Map()
  for (const p of plugins) for (const t of p.tools) if (!owner.has(t)) owner.set(t, p.dirName)
  const L = []

  L.push("## 2. 能力矩阵（每条能力 · 用户可见语义）");
  L.push("");
  L.push(
    "> 本节是**能力面的唯一 owner**：每条能力是什么、用户与模型因此看到什么，一句话只在这里出现一次。" +
      "语义全部**从代码提取**，人不在表里维护任何一句——" +
      "工具取 `CapabilityMeta.description`（模型与前端看到的原文）与同体的" +
      "`category` / `risk`，VDFS 操作取 `VDFS_*` 常量上方的文档注释，" +
      "路由取插件 `route()` 里那条 `match` 臂上方的注释。" +
      "覆盖面（有哪些插件、哪些配置面）见 §1。",
  );
  L.push(">");
  L.push("> **读表须知**：");
  L.push(">");
  L.push("> - `—` = **代码里没有逐条声明**（不是「没有这一项」）。风险未声明取" +
    "`CapabilityRiskLevel` 的默认档 `Medium`——默认值在 core，" +
    "这里不逐条复述，只说明空格的含义；");
  L.push("> - 描述尾部的 `…` = 该描述在运行期由 `format!` 拼装，表中给的是它的**静态开头一段**" +
    "（其余部分只有运行期才知道，如可用智能体清单），不是取全了；");
  L.push("> - `vdfs_*` 工具的「类别」列整族为空：类别由 `plugins/vdfs/tools/mod.rs::tool()` " +
    "统一置为 `Resource`，不在每条表项里逐条写；");
  L.push("> - 本节**不**覆盖路由的调用方与契约——那归 " + refLink('RE-003-ROUTES', '003-ROUTES.md') + "；" +
    "也不覆盖「测到了什么」——那归 " + refLink('RE-005-e2e覆盖矩阵', 'RE-005-e2e覆盖矩阵') + "。");
  L.push("");

  // ── 2.1 LLM 可见工具 ──
  const tools = [...cap.tools].sort(([a], [b]) => (a < b ? -1 : 1))
  L.push("### 2.1 LLM 可见工具");
  L.push("");
  L.push("<!-- list:tools owner -->");
  L.push("");
  L.push(`共 ${tools.length} 个（短名）。工具对模型的**全名** = \`<挂载点>/<短名>\`（\`traverse\` 注册时拼装）。`);
  L.push("");
  L.push("| 工具（短名） | 贡献插件 | 用户/模型可见语义（`CapabilityMeta.description`） | 类别 | 风险 |");
  L.push("|---|---|---|---|---|");
  for (const [name, v] of tools) {
    const plugin = owner.get(name);
    L.push(
      `| \`${name}\` | ${plugin ? `\`${plugin}\`` : UNKNOWN} | ${cell(maskRoot(v.description ?? UNKNOWN))} | ` +
        `${v.category ?? UNKNOWN} | ${v.risk ?? UNKNOWN} |`,
    );
  }
  L.push("");
  L.push("> **动态项（静态抽取拿不到，不要当成「没有」）**：");
  for (const [plugin, note] of Object.entries(DYNAMIC_TOOLS)) {
    L.push(`> - \`${plugin}\`：${note}。`);
  }
  L.push("> - 运行时权威来源：`traverse(\"available_tools\")` 广播进 `Registry`（收集口）的那份集合。");
  L.push("");

  // ── 2.2 VDFS 操作 ──
  L.push("### 2.2 VDFS 操作");
  L.push("");
  L.push("<!-- list:vdfs-ops owner -->");
  L.push("");
  L.push(
    `共 ${cap.vdfsOps.length} 个，取自 \`plugins/vdfs/protocol.rs::VDFS_OPS\`。` +
      "两条链路（前端 vs LLM 工具）不是一一对应，见 §3.2。",
  );
  L.push("");
  L.push("| 操作 | 一句话定义（常量上的文档注释） |");
  L.push("|---|---|");
  for (const op of cap.vdfsOps) {
    L.push(`| \`${op}\` | ${cell(maskRoot(cap.vdfsOpDocs.get(op) ?? UNKNOWN))} |`);
  }
  L.push("");

  // ── 2.3 前端 / 宿主路由 ──
  const routes = [...new Set(plugins.flatMap((p) => p.routes))].sort()
  L.push("### 2.3 前端 / 宿主路由");
  L.push("");
  L.push("<!-- list:routes owner -->");
  L.push("");
  L.push(
    `共 ${routes.length} 条静态可提取。§1 的「自有路由」格报的是同一份提取的**条数**（插件视角），` +
      "本表是**路由视角**——两者各答一问，不重复列同一批地址。",
  );
  L.push("");
  L.push("| 路由 | 一句话职责（`route()` 臂上的注释） |");
  L.push("|---|---|");
  for (const r of routes) {
    L.push(`| \`${r}\` | ${cell(maskRoot(cap.routeDocs.get(r) ?? UNKNOWN))} |`);
  }
  L.push("");
  L.push(
    "> 运行期动态分发的地址（`local/<工具短名>`、`vdfs/<操作>`、容器挂载名）不在这一节——" +
      "它们没有可静态枚举的臂，只在 §1 出现；`vdfs/*` 的逐条定义见 §2.2。",
  );
  L.push("");
  return L.join("\n")
}

// ================= 事实描述符（由 `scripts/gen-facts.mjs` 遍历调用） =================
//
// 溯源行（`> 生成时间：… UTC · 源：git rev-parse HEAD = …`）的剔除与幂等落盘
// 统一在 `facts/_lib.mjs`：溯源行**不参与**「内容变没变」的判定，否则 `gen`
// 就不再是函数——同一份代码每次运行都产出不同内容，autoWork 比对内容哈希，
// 于是每次门禁都报「修复了 1 个文件」，而 CI 只能判红，**红的原因是时间戳**。
export default {
  id: "current",
  title: "读侧结构事实表（插件 / 工具 / 路由 / 存储 / 规模）",
  out: OUT_REL,
  render,
};
