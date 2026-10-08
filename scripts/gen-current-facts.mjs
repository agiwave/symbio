#!/usr/bin/env node
/**
 * 读侧事实表生成器 —— docs/CURRENT.md
 *
 * 动机（真实 agent 会话自评估驱动）：审查本仓库的 agent 反复反馈"现在是什么"
 * 这张事实表不存在——只有「写侧论证」（ADR 全在讲为什么）与"会漂移的手抄清单"
 * （README / ROUTES.md 都曾漂移）。于是核对一条事实要 1-3 轮往返，核对成本高到
 * agent 选择"不核对、用自信语气包装未验证结论"。
 *
 * 本脚本**只从代码抽取**（提取，不手写）：每一行都能追到源文件，
 * 因此它不会像手抄清单那样漂移。它只回答"现在是什么"（结构）；
 * "为什么"仍看 DECISIONS.md（索引）+ decisions/*.md（按域分的正文分册）。
 *
 * 用法：
 *   node scripts/gen-current-facts.mjs            # 生成 docs/CURRENT.md
 *   node scripts/gen-current-facts.mjs --check    # 与现有文件比对，漂移则非零退出（CI 用）
 *
 * 口径（诚实优先于完整）：
 *   - 静态可提取的一律提取；提取不到的**标注为动态**，绝不臆造
 *     （如 `mcp` 运行期桥接的工具、`local/shell` 按操作系统取名的工具）。
 *   - 路由：本表只列**静态可提取**的字符串臂，权威清单仍是
 *     `docs/reference/ROUTES.md`（两者交叉核对构成漂移门禁）。
 *   - 不猜 VDFS 挂载点的资源存储选型（`vdfs_service` 的 Single/Dir/Memory 是
 *     实现细节，见 `docs/design/vdfs.md` §11）。
 */

import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { scopeRow as sharedScopeRow } from "./line-count.mjs";
import { stripComments } from "./rust-scan.mjs";
// 路由提取只有一份实现（`route-facts.mjs`），与前端常量生成器共用——
// 「什么算一条路由」这条判据若有两遍，它们会在没人注意时分叉。
import { pluginSources, extractRouteArms, vdfsOps } from "./route-facts.mjs";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(SCRIPT_DIR, "..");
const PLUGINS_DIR = path.join(ROOT, "symbio", "src", "plugins");
// 插件工厂 id 的 owner 是 `plugin` 域（`symbio_core/plugin/ids.rs`）——它们描述的是
// 「哪个插件」，不是「哪个键」，故不随 `keys` 域走（见 `symbio_core/README.md` §1.2
// 的「放置也是同一条规则的一部分」）。
const IDS_FILE = path.join(ROOT, "symbio", "src", "symbio_core", "plugin", "ids.rs");
const OUT = path.join(ROOT, "docs", "CURRENT.md");

const VDFS_FS_FILE = path.join(PLUGINS_DIR, "vdfs", "fs.rs");

/**
 * 从 `pub const NAME: &str = "VALUE"` 提取字面量。
 * 用于生成器**不手写**事实（如 VDFS 根名），保持 CURRENT.md 与真相单源。
 * 输出到文档时替换为 `<vdfs_root>` 占位（见 S-010：根名字面量只归 vdfs 插件）。
 */
function vdfsAddrRoot() {
  const src = readFileSync(VDFS_FS_FILE, "utf8");
  const m = src.match(/pub const VDFS_ADDR_ROOT:\s*&str\s*=\s*"([^"]+)"/);
  if (!m) throw new Error("cannot extract VDFS_ADDR_ROOT from plugins/vdfs/fs.rs");
  return m[1];
}

/** 收集文件内的 `const X: &str = "…"` 常量（用于解析 `name: CONST.to_string()`） */
function parseConsts(txt, map) {
  for (const mm of txt.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    map.set(mm[1], mm[2]);
  }
  return map;
}

/** ids.rs：`pub const PLUGIN_ID_LOCAL: &str = "local";` → Map 常量名 → 值 */
function parseIds(txt) {
  const m = new Map();
  for (const mm of txt.matchAll(/pub const (PLUGIN_[A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    m.set(mm[1], mm[2]);
  }
  return m;
}

/** 解析 `PluginMeta::new(X` / `register_vdfs_provider(X` 的第一个实参 */
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
  // `name: CONST_NAME.to_string()` —— 名字由常量给出（如 context_compact）
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
  return [...out].sort();
}

/** 核心 trait 白名单：只统计真正定义插件形态的 trait */
const CORE_TRAITS = ["Plugin", "VdfsProvider", "Capability", "ModelProvider", "ConfigurableVisitor"];

/** 单个插件的静态事实 */
function analyzePlugin(dirName, ids) {
  const dir = path.join(PLUGINS_DIR, dirName);
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
    for (const mm of t.matchAll(/register_vdfs_provider\(\s*([^,\n]+)/g)) {
      mounts.add(resolveArg(mm[1], ids));
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
    sharedScopeRow(ROOT, "symbio/src", [".rs"]),
    sharedScopeRow(ROOT, "cli/src", [".rs"]),
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
 * 权威来源是 `cli/src/args.rs` 的 `HELP` 常量（无子命令树、不依赖 clap）。
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
 * 只指向 ROUTES.md、不再自列端点（早年 README 写 `/api/route`、`/api/ws` 而代码
 * 是 `/api/v1/invoke`、`/api/v1/health`，是文档腐烂的活案例）。
 */
function gatewayEndpoints() {
  const file = path.join(ROOT, "symbio", "src", "plugins", "gateway", "server.rs");
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

const TOOL_NOTES = {
  agent_identity: "取回当前智能体完整人格文本",
  agent_run: "委托子智能体",
  ask_user: "向用户提问",
  codebase_search: "语义代码搜索",
  content_search: "正则内容搜索（ripgrep 库）",
  todo_write: "任务清单（`LastOnly` 保留策略）",
  heartbeat: "会话心跳配置",
  read_skill: "读取技能定义",
  http_request: "HTTP 请求",
  web_fetch: "网页抓取",
  web_search: "网页搜索",
  context_compact: "主动压缩上下文（开关打开时才暴露给模型）",
};

/** 静态抽取拿不到的工具来源：明确标注，绝不臆造 */
const DYNAMIC_TOOLS = {
  local:
    "`shell` 族工具名按操作系统取（Windows = `cmd`），静态抽取只见占位名 → 以运行时 `traverse(\"available_tools\")` 为准",
  mcp: "桥接工具在运行期按已连接 server 动态生成，无法静态枚举",
};

/** 路由按运行期规则分发、无法静态枚举的插件：标注而不是留空 */
const DYNAMIC_ROUTES = {
  local: "`local/<工具短名>`——按已注册工具名分发（与 §2 的工具清单同一份集合）",
  // 操作数从协议源码动态推导（VDFS_OPS 增删时不再漂移）
  vdfs: `\`vdfs/<操作>\`——按 \`VDFS_OPS\` 校验后分发（见 §3.2，${
    vdfsOps(ROOT).length
  } 个操作）`,
  composite: "容器：按配置挂载的子插件名分发，运行期动态",
  agent: "已无自有路由（一律 `NotFound` 并指引到 `<根>/agent`）",
  model: "已无自有路由（`execute_turn` 由 session 直连调用）",
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
  const vdfsOpList = vdfsOps(ROOT);
  const pluginDirs = readdirSync(PLUGINS_DIR, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
  const plugins = pluginDirs.map((d) => analyzePlugin(d, ids));
  // 静态可提取的地址全集（§3.1 报条数用；与前端生成物同一份提取口径）
  const staticRoutes = [...new Set(plugins.flatMap((p) => p.routes))].sort();

  const L = [];
  L.push("# Symbio 当前事实表（自动生成）");
  L.push("");
  L.push("> ⚠️ **本文件由 `scripts/gen-current-facts.mjs` 从代码提取生成，请勿手改。**");
  L.push("> 改了代码不用手动重跑——门禁会**自动重新生成并暂存**（见 `scripts/gate.d/60-facts.mjs`）。");
  L.push(">");
  L.push("> 本表回答「**现在是什么**」（结构；代码投影，不会漂移）；");
  L.push("> 「**为什么这样设计**」看 [DECISIONS.md](./DECISIONS.md)（索引）+ `decisions/*.md`（分域正文）；");
  L.push("> 路由权威清单看 [reference/ROUTES.md](./reference/ROUTES.md)（与本表交叉核对）。");
  L.push("");
  L.push("<!-- AUTO-GENERATED by scripts/gen-current-facts.mjs — do not edit by hand -->");
  L.push("");

  // ── §1 插件总表 ─────────────────────────────────────────────────────
  L.push("## 1. 插件总表（`symbio/src/plugins/`）");
  L.push("");
  L.push(
    "| 插件目录 | 注册名 | VDFS 挂载点 | 自有路由（静态可提取） | 实现的核心 trait | 配置文件 | 模块 README |"
  );
  L.push("|---|---|---|---|---|---|---|");
  for (const p of plugins) {
    const routeCell = p.routes.length
      ? fmtList(p.routes)
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
  L.push(">   插件经 `Plugin::get_vfs_provider` 把自己的视图交给容器（**系统 / 前端链路**，");
  L.push(">   容器聚合）；**LLM 链路**另经 `CapabilityVisitor::register_vdfs_provider(目录名, provider)`");
  L.push(">   按名注册（可控挂载机制，预留按作用域裁剪），两条通道目录名同一份。");
  L.push(">   容器（`composite`）自身即 `<根>` 的服务者，其挂载点是运行期动态。");
  L.push(">   资源存储的**选型**（`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs`）是实现细节，不在本表出现。");
  L.push("> - **自有路由** = `async fn route()` 体内 `match` 臂的字符串（臂是**相对路径**，");
  L.push(">   容器已剥掉首段，故此处补回**插件目录名**——容器按目录名建实例表并按它分发，");
  L.push(">   目录名才是真正的路由前缀；`PluginMeta` 首参不参与路由——它只是**出厂 id**，");
  L.push(">   插件**身份**取自 `PLUGIN.yml`（见 [DECISIONS.md](./DECISIONS.md) ADR-032）。");
  L.push(">   标「（动态）」的是按运行期规则分发、无法静态枚举的。");
  L.push(">   漏项与歧义以 [ROUTES.md](./reference/ROUTES.md) 为准。");
  L.push("> - **配置文件** = 该插件调用过 `capability_announce_configurable`（配置就是 `<根>/<挂载点>/PLUGIN.yml`，");
  L.push(">   读写走 `vdfs/read` / `vdfs/write`，**没有配置专用路由**）。");
  L.push("");

  // ── §2 LLM 可见工具 ─────────────────────────────────────────────────
  L.push("## 2. LLM 可见工具（`CapabilityMeta.name` 短名 → 贡献插件）");
  L.push("");
  L.push("工具对模型的**全名** = `<挂载点>/<短名>`（`traverse` 注册时拼装），故下表给短名。");
  L.push("");
  L.push("| 工具（短名） | 贡献插件 | 类别 / 备注 |");
  L.push("|---|---|---|");
  for (const p of plugins) {
    for (const t of p.tools) {
      L.push(`| \`${t}\` | \`${p.dirName}\` | ${TOOL_NOTES[t] ?? ""} |`);
    }
  }
  L.push("");
  L.push("> **动态项（静态抽取拿不到，不要当成「没有」）**：");
  for (const [plugin, note] of Object.entries(DYNAMIC_TOOLS)) {
    L.push(`> - \`${plugin}\`：${note}。`);
  }
  L.push("> - 运行时权威来源：`traverse(\"available_tools\")` 广播进 `CapabilityVisitor` 的那份集合。");
  L.push("");

// ── §3 协议速查 ─────────────────────────────────────────────────────
  L.push("## 3. 协议速查");
  L.push("");
  L.push("### 3.1 前端 / 宿主路由");
  L.push("");
  L.push("```");
  L.push("{container}/{plugin}/{action}     例：worker/session/chat/send（worker 可省略）");
  L.push("```");
  L.push("");
  L.push("- 完整路由清单见 [reference/ROUTES.md](./reference/ROUTES.md)。");
  L.push(
    `- **前端侧常量**：\`tauri/src/constants/routes.gen.ts\`——${staticRoutes.length} 条静态可提取地址，` +
      "由后端 `route()` 的 `match` 臂**生成**（`scripts/gen-routes-ts.mjs`），不是手写登记处；" +
      "前端在别处写死同一条地址由 `mechanism-audit` 的 M-008 判红。",
  );
  L.push("");
  L.push("### 3.2 VDFS 操作（`plugins/vdfs/protocol.rs::VDFS_OPS`）");
  L.push("");
  L.push(`- **前端链路**（${vdfsOpList.length} 个，计数有测试锁死）：${fmtList(vdfsOpList)}`);
  const vdfsPlugin = plugins.find((p) => p.dirName === "vdfs");
  const llmTools = vdfsPlugin ? vdfsPlugin.tools.filter((t) => t.startsWith("vdfs_")) : [];
  L.push(`- **LLM 工具链路**（${llmTools.length} 个）：${fmtList(llmTools)}`);
  L.push("  （`watch` / `unwatch` / `action` 不经工具暴露，故两条链路不是一一对应）");
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
    "| 会话与其消息 | `<homedir>/session/<id>/{session.json,messages.json}` | **单一具体类型** `SessionStore`（持久=磁盘布局 / 临时=进程内驻留）。曾有 `store_kind` × file/sqlite/memory 三后端选型，**已删除** |"
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
  // 这里曾长期写着 `<homedir>/plugins/<插件>/PLUGIN.yml`，与代码和 CONFIGURATION.md
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
  L.push(
    `- **CLI 面**：进程选项 ${cli.longs.length} 个长 + ${cli.shorts.length} 个短（${fmtList(
      cli.longs
    )}）；交互模式内置命令 ${cli.repl.length} 个（${fmtList(
      cli.repl
    )}）。无子命令树、不依赖 clap，权威来源是 \`cli/src/args.rs\` 的 \`HELP\`。`
  );
  L.push("");

  // ── §6 生成信息 ─────────────────────────────────────────────────────
  L.push("---");
  L.push("");
  const stamp = new Date().toISOString().slice(0, 19).replace("T", " ");
  L.push(`> 生成时间：${stamp} UTC · 源：\`git rev-parse HEAD\` = \`${headSha()}\``);
  L.push("");
  return L.join("\n");
}

// ================= 入口 =================

// 溯源行（`> 生成时间：… UTC · 源：git rev-parse HEAD = …`）**不参与**「内容变没变」的判定。
//
// 它是随运行时间（以及当前 HEAD）变的。若让它决定写不写，本脚本就**不是函数**了：
// 同一份代码，每次运行都产出一份不同的文件。后果不只是噪音——
// 门禁把这个脚本当**确定性的机械工作**自动执行（`gate.d/_shared.autoWork`），
// 比对的是**内容哈希**：于是每次门禁都会报「修复了 1 个文件」，
// 而 CI 无法提交、只能判红，**红的原因是时间戳，不是漂移**。
//
// 所以：代码派生内容没变 ⇒ **整份沿用已有文件**（连同它原来的溯源行），
// 使「同输入 ⇒ 同输出」逐字节成立。溯源行因此表示的是
// 「**内容最后一次真正变化**是在何时、基于哪个 commit」，比「最后一次运行时间」更有意义。
const stripStamp = (s) =>
  s
    .split("\n")
    .filter((l) => !l.startsWith("> 生成时间："))
    .join("\n");

const content = render();
const isCheck = process.argv.includes("--check");

if (isCheck) {
  if (!existsSync(OUT)) {
    console.error(" docs/CURRENT.md 不存在——先运行 `node scripts/gen-current-facts.mjs`");
    process.exit(1);
  }
  // 生成时间行随时间变化，比对时剔除；其余任何差异都是真漂移
  if (stripStamp(readFileSync(OUT, "utf8")) !== stripStamp(content)) {
    console.error("❌ docs/CURRENT.md 与代码漂移——重新运行 `node scripts/gen-current-facts.mjs`");
    process.exit(1);
  }
  console.log(`✅ docs/CURRENT.md 与代码一致（${content.split("\n").length} 行）`);
} else {
  const existing = existsSync(OUT) ? readFileSync(OUT, "utf8") : null;
  if (existing !== null && stripStamp(existing) === stripStamp(content)) {
    console.log(`✅ docs/CURRENT.md 已是最新（${content.split("\n").length} 行，未改写）`);
  } else {
    writeFileSync(OUT, content, "utf8");
    console.log(`✅ 已生成 docs/CURRENT.md（${content.split("\n").length} 行）`);
  }
}
