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
 * "为什么"仍看 DECISIONS.md。
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

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(SCRIPT_DIR, "..");
const PLUGINS_DIR = path.join(ROOT, "symbio", "src", "plugins");
const IDS_FILE = path.join(ROOT, "symbio", "src", "symbio_core", "ids.rs");
const VDFS_PROTOCOL_FILE = path.join(PLUGINS_DIR, "vdfs", "protocol.rs");
const OUT = path.join(ROOT, "docs", "CURRENT.md");

/** 递归收集插件源码（排除测试文件与 docs 目录：测试里的 meta 不是生产事实） */
function collectRs(dir) {
  const out = [];
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      if (ent.name === "docs") continue;
      out.push(...collectRs(p));
    } else if (
      ent.name.endsWith(".rs") &&
      !ent.name.endsWith(".test.rs") &&
      ent.name !== "tests.rs"
    ) {
      out.push(p);
    }
  }
  return out;
}

/**
 * 去注释（字符串感知）。
 *
 * 朴素的 `//.*$` 正则会砍掉**字符串里的** `//`（`"https://api.openai.com/v1"`
 * 会被截断），既破坏事实提取，也会让后续花括号配对错位——故按字符扫描，
 * 遇到字符串字面量整体复制、只在串外识别注释。
 */
function stripComments(txt) {
  let out = "";
  for (let p = 0; p < txt.length; p++) {
    const c = txt[p];
    if (c === '"') {
      const end = skipString(txt, p);
      // 进度保证：解析结果必须至少吃掉当前字符
      if (end > p) {
        out += txt.slice(p, end + 1);
        p = end;
        continue;
      }
    } else if (c === "/" && txt[p + 1] === "/") {
      const end = txt.indexOf("\n", p);
      if (end < 0) break;
      out += "\n";
      p = end;
      continue;
    } else if (c === "/" && txt[p + 1] === "*") {
      const end = txt.indexOf("*/", p + 2);
      if (end < 0) break;
      p = end + 1;
      continue;
    }
    out += c;
  }
  return out;
}

/**
 * 去掉 `#[cfg(test)]` 测试模块（`#[cfg(test)] mod tests { … }`）。
 *
 * 为什么需要：测试里的 `CapabilityMeta` / `PluginMeta`（如 vdfs/host.rs 的假容器
 * `PluginMeta::new("fake", …)`）不是生产事实，不剔除会污染事实表
 * （实测踩坑：`vdfs` 的注册名被抽成 `fake`）。
 *
 * 为什么不能"从第一个 `#[cfg(test)]` 直接截断到文件尾"：仓库里存在
 * **测试模块之后还有生产代码**的文件（如 `model/plugin.rs`：`mod tests` 在中段，
 * `traverse` 里的挂载点注册在其后）——截断会把生产事实一起丢掉
 * （实测踩坑：`model` 的 `.vdfs/model` 挂载点消失）。
 *
 * 故按**花括号配对**精确剔除模块体，并对字符串字面量做感知（测试代码里
 * `format!("{{}}")` 这类字面量括号会让朴素配对错位）。
 */
function stripTestModules(txt) {
  const MARKER = "#[cfg(test)]";
  let out = "";
  let i = 0;
  for (;;) {
    const idx = txt.indexOf(MARKER, i);
    if (idx < 0) {
      out += txt.slice(i);
      return out;
    }
    out += txt.slice(i, idx);
    const after = txt.slice(idx + MARKER.length);
    const modHead = after.match(/^\s*(?:#\[[^\]]*\]\s*)*mod\s+[A-Za-z0-9_]+\s*\{/);
    // 进度保证：无论匹配是否成功，i 都必须严格前进（否则死循环）
    if (modHead) {
      const open = idx + MARKER.length + modHead[0].length - 1;
      i = Math.max(matchBrace(txt, open) + 1, open + 1);
    } else {
      // 不是模块（如 `#[cfg(test)] use …;`）——只吞掉标记本身，保留后续代码
      i = idx + MARKER.length;
    }
  }
}

/** 从 `{` 起做字符串感知的花括号配对，返回对应 `}` 的下标（失败返回文本末尾） */
function matchBrace(txt, openIdx) {
  let depth = 0;
  for (let p = openIdx; p < txt.length; p++) {
    const c = txt[p];
    if (c === '"') {
      p = skipString(txt, p);
      continue;
    }
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return p;
    }
  }
  return txt.length - 1;
}

/**
 * 跳过字符串字面量，返回闭引号（或原始串收尾分隔符末位）的下标。
 *
 * 只**向前**扫描：返回值恒 ≥ `start`，调用方据此推进扫描位置；
 * 一旦返回更小的下标（曾用"向前 lastIndexOf 找 r" 的实现），
 * 外层扫描就会原地打转（实测踩坑：生成器死循环）。
 */
function skipString(txt, start) {
  // 原始字符串判定：开引号紧跟在 `r` 或 `r###` 之后（`r"…"` / `r#"…"#` / `br#"…"#`）
  let hashes = 0;
  let j = start - 1;
  while (j >= 0 && txt[j] === "#") {
    hashes++;
    j--;
  }
  if (j >= 0 && txt[j] === "r") {
    const close = '"' + "#".repeat(hashes);
    const end = txt.indexOf(close, start + 1);
    return end < 0 ? txt.length - 1 : end + close.length - 1;
  }
  for (let p = start + 1; p < txt.length; p++) {
    if (txt[p] === "\\") {
      p++;
      continue;
    }
    if (txt[p] === '"') return p;
  }
  return txt.length - 1;
}

/** 收集文件内的 `const X: &str = "…"` 常量（用于解析 `name: CONST.to_string()`） */
function parseConsts(txt, map) {
  for (const mm of txt.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    map.set(mm[1], mm[2]);
  }
  return map;
}

/** ids.rs：`pub const PLUGIN_LOCAL: &str = "local";` → Map 常量名 → 值 */
function parseIds(txt) {
  const m = new Map();
  for (const mm of txt.matchAll(/pub const (PLUGIN_[A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    m.set(mm[1], mm[2]);
  }
  return m;
}

/** vdfs/protocol.rs：协议操作常量 + `VDFS_OPS` 清单（按声明顺序） */
function parseVdfsOps(txt) {
  const consts = new Map();
  for (const mm of txt.matchAll(/pub const (VDFS_[A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    consts.set(mm[1], mm[2]);
  }
  const block = txt.match(/pub const VDFS_OPS\s*:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/);
  if (!block) return [];
  return [...block[1].matchAll(/\b(VDFS_[A-Z_]+)\b/g)].map((m) => consts.get(m[1])).filter(Boolean);
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

/**
 * 从 `async fn route(…)` 体内提取路由臂。
 *
 * 只认 `match` 臂形态（`"a/b" | "c" => …` 左侧的字符串字面量），
 * 不整段抓字符串——后者会把 `get("approved")` 这类参数名当成路由（实测踩坑）。
 *
 * 臂是**相对路径**：容器（composite）先剥掉首段，插件只分发剩下的部分，
 * 故渲染时要补回插件注册名（`home` 是根容器，臂已是全名，不补）。
 */
function extractRouteArms(t, pluginName) {
  const out = new Set();
  const ri = t.indexOf("async fn route(");
  if (ri < 0) return [];
  const rest = t.slice(ri);
  const nextFn = rest.indexOf("async fn ", 20);
  const chunk = rest.slice(0, nextFn < 0 ? Math.min(6000, rest.length) : nextFn);
  for (const line of chunk.split("\n")) {
    const eq = line.indexOf("=>");
    if (eq < 0) continue;
    const left = line.slice(0, eq);
    for (const mm of left.matchAll(/"([a-z][a-z0-9_/-]*)"/g)) {
      if (mm[1] === "_") continue;
      const arm = mm[1];
      const full =
        pluginName && pluginName !== "home" && !arm.startsWith(`${pluginName}/`)
          ? `${pluginName}/${arm}`
          : arm;
      out.add(full);
    }
  }
  return [...out].sort();
}

/** 核心 trait 白名单：只统计真正定义插件形态的 trait */
const CORE_TRAITS = ["Plugin", "VdfsProvider", "Capability", "ModelProvider", "ConfigurableVisitor"];

/** 单个插件的静态事实 */
function analyzePlugin(dirName, ids) {
  const dir = path.join(PLUGINS_DIR, dirName);
  const contents = collectRs(dir).map((f) =>
    stripTestModules(stripComments(readFileSync(f, "utf8")))
  );
  const consts = new Map();
  for (const t of contents) parseConsts(t, consts);

  let metaName = null;
  const mounts = new Set();
  const tools = new Set();
  const traits = new Set();
  const routes = new Set();
  let hasConfig = false;

  for (const t of contents) {
    if (!metaName) {
      const mm = t.match(/PluginMeta::new\(([^,\n]+)/);
      if (mm) metaName = resolveArg(mm[1], ids);
    }
    for (const mm of t.matchAll(/register_vdfs_provider\(\s*([^,\n]+)/g)) {
      mounts.add(resolveArg(mm[1], ids));
    }
    if (t.includes("announce_configurable(")) hasConfig = true;
    for (const n of extractToolNames(t, consts)) tools.add(n);
    for (const mm of t.matchAll(
      /impl\s+(?:<[^>]*>\s*)?([A-Z][A-Za-z0-9_]*)(?:<[^>]*>)?\s+for\s+/g
    )) {
      if (CORE_TRAITS.includes(mm[1])) traits.add(mm[1]);
    }
    for (const arm of extractRouteArms(t, metaName)) routes.add(arm);
  }

  return {
    dirName,
    metaName,
    mounts: [...mounts].sort(),
    tools: [...tools].sort(),
    traits: [...traits].sort(),
    routes: [...routes].sort(),
    hasConfig,
    hasReadme: existsSync(path.join(dir, "README.md")),
  };
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
  vdfs: "`vdfs/<操作>`——按 `VDFS_OPS` 校验后分发（见 §3.2，13 个操作）",
  composite: "容器：按配置挂载的子插件名分发，运行期动态",
  agent: "已无自有路由（一律 `NotFound` 并指引到 `.vdfs/agent`）",
  model: "已无自有路由（`execute_turn` 由 session 直连调用）",
};

const fmtList = (items) => (items.length ? items.map((x) => `\`${x}\``).join(" · ") : "—");
const fmtMounts = (items) =>
  items.length
    ? items.map((m) => (m.startsWith("（") ? m : `.vdfs/${m}`)).join(" · ")
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
  const vdfsOps = parseVdfsOps(readFileSync(VDFS_PROTOCOL_FILE, "utf8"));
  const pluginDirs = readdirSync(PLUGINS_DIR, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
  const plugins = pluginDirs.map((d) => analyzePlugin(d, ids));

  const L = [];
  L.push("# Symbio 当前事实表（自动生成）");
  L.push("");
  L.push("> ⚠️ **本文件由 `scripts/gen-current-facts.mjs` 从代码提取生成，请勿手改。**");
  L.push("> 改了代码就重跑 `node scripts/gen-current-facts.mjs`；CI 跑 `--check` 防漂移。");
  L.push(">");
  L.push("> 本表回答「**现在是什么**」（结构；代码投影，不会漂移）；");
  L.push("> 「**为什么这样设计**」看 [DECISIONS.md](./DECISIONS.md)；");
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
      `| \`${p.dirName}\` | \`${p.metaName ?? "—"}\` | ${fmtMounts(p.mounts)} | ${routeCell} | ${fmtList(
        p.traits
      )} | ${p.hasConfig ? "✓" : "—"} | ${p.hasReadme ? "✓" : "**缺**"} |`
    );
  }
  L.push("");
  L.push("> 读表须知：");
  L.push("> - **挂载点** = 该插件在 `traverse` 里 `register_vdfs_provider(目录名, provider)` 的目录名；");
  L.push(">   容器（`composite`）按子插件注册名合成目录树，其自身挂载点是运行期动态。");
  L.push(">   资源存储的**选型**（`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs`）是实现细节，不在本表出现。");
  L.push("> - **自有路由** = `async fn route()` 体内 `match` 臂的字符串（臂是**相对路径**，");
  L.push(">   容器已剥掉首段，故此处补回注册名）；标「（动态）」的是按运行期规则分发、无法静态枚举的。");
  L.push(">   漏项与歧义以 [ROUTES.md](./reference/ROUTES.md) 为准。");
  L.push("> - **配置文件** = 该插件调用过 `announce_configurable`（配置就是 `.vdfs/<挂载点>/PLUGIN.yml`，");
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
  L.push("完整路由清单见 [reference/ROUTES.md](./reference/ROUTES.md)。");
  L.push("");
  L.push("### 3.2 VDFS 操作（`plugins/vdfs/protocol.rs::VDFS_OPS`）");
  L.push("");
  L.push(`- **前端链路**（${vdfsOps.length} 个，计数有测试锁死）：${fmtList(vdfsOps)}`);
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
    "| 资源型插件条目（agent / model / mcp / skill / setting …） | `<homedir>/plugins/<类别>/<id>/<主文件>` | `providers/vdfs_service/` 三型：`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs` |"
  );
  L.push(
    "| 会话与其消息 | `<homedir>/plugins/session/<id>/{session.json,messages.json}` | **单一具体类型** `SessionStore`（持久=磁盘布局 / 临时=进程内驻留）。曾有 `store_kind` × file/sqlite/memory 三后端选型，**已删除** |"
  );
  L.push("| Agent bundle | bundle 目录（工作区级 + 全局级双层） | `BundleStore` 自管，不经 `vdfs_service` |");
  L.push(
    "| 插件配置（含会话配置） | `<homedir>/plugins/<插件>/PLUGIN.yml`（系统级在 `<homedir>/PLUGIN.yml`） | `ConfigFile` 自读写，**无第二种后端、无第二条配置协议** |"
  );
  L.push("");

  // ── §5 生成信息 ─────────────────────────────────────────────────────
  L.push("---");
  L.push("");
  const stamp = new Date().toISOString().slice(0, 19).replace("T", " ");
  L.push(`> 生成时间：${stamp} UTC · 源：\`git rev-parse HEAD\` = \`${headSha()}\``);
  L.push("");
  return L.join("\n");
}

// ================= 入口 =================

const content = render();
const isCheck = process.argv.includes("--check");

if (isCheck) {
  if (!existsSync(OUT)) {
    console.error(" docs/CURRENT.md 不存在——先运行 `node scripts/gen-current-facts.mjs`");
    process.exit(1);
  }
  // 生成时间行随时间变化，比对时剔除；其余任何差异都是真漂移
  const strip = (s) =>
    s
      .split("\n")
      .filter((l) => !l.startsWith("> 生成时间："))
      .join("\n");
  if (strip(readFileSync(OUT, "utf8")) !== strip(content)) {
    console.error("❌ docs/CURRENT.md 与代码漂移——重新运行 `node scripts/gen-current-facts.mjs`");
    process.exit(1);
  }
  console.log(`✅ docs/CURRENT.md 与代码一致（${content.split("\n").length} 行）`);
} else {
  writeFileSync(OUT, content, "utf8");
  console.log(`✅ 已生成 docs/CURRENT.md（${content.split("\n").length} 行）`);
}
