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
const IDS_FILE = path.join(ROOT, "symbio", "src", "symbio_core", "keys", "ids.rs");
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
 * （实测踩坑：`model` 的 `<根>/model` 挂载点消失；`<根>` = vdfs 插件声明的挂载根名）
 *
 * 故按**花括号配对**精确剔除模块体，并对字符串字面量做感知（测试代码里
 * `format!("{{}}")` 这类字面量括号会让朴素配对错位）。
 */
/**
 * 收集 `#[cfg(test)]` 测试模块占据的字符区间 `[start, end)`。
 *
 * 扫描逻辑的**唯一真源**：`stripTestModules`（提取事实时剔除测试代码）与
 * `splitRustLines`（统计行数时把内联测试归属到"测试"列）都建立在它之上。
 */
function testModuleSpans(txt) {
  const MARKER = "#[cfg(test)]";
  const spans = [];
  let i = 0;
  for (;;) {
    const idx = txt.indexOf(MARKER, i);
    if (idx < 0) return spans;
    const after = txt.slice(idx + MARKER.length);
    const modHead = after.match(/^\s*(?:#\[[^\]]*\]\s*)*mod\s+[A-Za-z0-9_]+\s*\{/);
    // 进度保证：无论匹配是否成功，i 都必须严格前进（否则死循环）
    if (modHead) {
      const open = idx + MARKER.length + modHead[0].length - 1;
      const end = Math.max(matchBrace(txt, open) + 1, open + 1);
      spans.push([idx, end]);
      i = end;
    } else {
      // 不是模块（如 `#[cfg(test)] use …;`）——只吞掉标记本身，保留后续代码
      i = idx + MARKER.length;
    }
  }
}

/** 剔除上面那些区间，其余文本原样保留（顺序拼接即等价于"挖掉"） */
function stripTestModules(txt) {
  const spans = testModuleSpans(txt);
  if (spans.length === 0) return txt;
  let out = "";
  let i = 0;
  for (const [s, e] of spans) {
    out += txt.slice(i, s);
    i = e;
  }
  return out + txt.slice(i);
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
 * 故渲染时要补回前缀。
 *
 * ## 前缀取**目录名**，不取 `PluginMeta::new` 的首参
 *
 * 容器按**目录名**建实例表并在 `route` 里按它分发（`composite.rs`「目录名 = 实例名」），
 * 所以目录名才是真正的路由前缀。`PluginMeta` 首参曾长期被当作前缀用，而它**不参与路由**
 * ——ADR-032 之后它只是**出厂 id**：身份取自 `PLUGIN.yml`，`composite/vdfs.rs` 只读它的
 * 「挂载点呈现」那部分（`order` / `hidden` / `root_access`）——`hook` 插件写成 `"hooks"`
 * 就由此产出了
 * `hooks/fire` 这类**不存在的路由**，并被本表与三处文档照抄。改用目录名后，
 * 「生成器说出的路由」与「容器真正认的路由」同源；`plugin-entry-audit.mjs` 的 E-001
 * 另外把「`PluginMeta` 首参 == 目录名」钉住，使两者不会再分叉。
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

/**
 * 「未接线」标记：模块级 `#![allow(dead_code)]`。
 *
 * 为什么需要：模块级地整体抑制 dead_code，等于自述「这块代码还没有接线」。
 * 它里面的 `CapabilityMeta { name: … }` 是**未接线的定义**，不是 LLM 可见工具；
 * 不排除就会让 §2 报出一个模型根本看不到的工具。
 *
 * 判据取**模块级属性**而不是猜注释文案——可用 grep 复核。当前全仓**无实例**
 * （唯一的 `plugins/local/ask_user.rs` 已于 2026-09-17 接线注册并由 §2 正常收录）；
 * 保留此判据是为了将来再出现未接线模块时自动生效。
 */
const UNWIRED_MARKER = "#![allow(dead_code)]";

/** 单个插件的静态事实 */
function analyzePlugin(dirName, ids) {
  const dir = path.join(PLUGINS_DIR, dirName);
  const contents = collectRs(dir)
    .map((f) => readFileSync(f, "utf8"))
    .filter((raw) => !raw.includes(UNWIRED_MARKER))
    .map((raw) => stripTestModules(stripComments(raw)));
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
    if (t.includes("announce_configurable(")) hasConfig = true;
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

// ================= 规模与宿主接缝的提取 =================

/** 目录遍历时跳过的名字（构建产物 / 依赖 / 版本库） */
const SCOPE_SKIP = ["target", "vendor", "node_modules", "dist", ".git"];

/** 递归收集指定后缀的文件（测试文件按 `*.test.rs` / `*.spec.*` 单独归类，不混进实现） */
function walkScope(dir, exts, isTest) {
  const out = [];
  let ents;
  try {
    ents = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const e of ents) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (SCOPE_SKIP.includes(e.name)) continue;
      out.push(...walkScope(p, exts, isTest));
      continue;
    }
    const test = e.name.endsWith(".test.rs") || e.name.endsWith(".spec.ts") || e.name === "tests.rs";
    if (!exts.some((x) => e.name.endsWith(x)) || test !== isTest) continue;
    out.push(p);
  }
  return out;
}

function countLines(files) {
  let n = 0;
  for (const f of files) n += readFileSync(f, "utf8").split("\n").length - 1;
  return n;
}

/**
 * `.rs` 文件的实现/测试行数归属。
 *
 * 为什么需要：`#[cfg(test)] mod tests { … }` 是**内联**在实现文件里的，按"整文件"
 * 归类会把测试行算进实现（实测：`symbio/src` 的实现行因此虚高约 7.5k，测试行虚低
 * 同量，会让人误判测试密度）。独立测试文件（`*.test.rs` / `tests.rs`）本来就被
 * `walkScope` 分开了，这里只处理内联模块。
 *
 * 返回仍按**行数**（换行符个数）计，与 `countLines` 同口径。
 */
function splitRustLines(files) {
  let impl = 0;
  let test = 0;
  for (const f of files) {
    const txt = readFileSync(f, "utf8");
    const total = txt.split("\n").length - 1;
    let inlineTest = 0;
    for (const [s, e] of testModuleSpans(txt)) {
      inlineTest += txt.slice(s, e).split("\n").length - 1;
    }
    impl += total - inlineTest;
    test += inlineTest;
  }
  return { impl, test };
}

/**
 * 一个统计范围：`dir` 相对仓库根，`exts` 参与统计的后缀。
 *
 * ⚠️ `dir` 会**原样进生成物**（§5.1 表格首列），故调用方必须给**正斜杠字面量**，
 * 不得用 `path.join` —— 后者在 Windows 上产出 `symbio\src`、在 Linux 上产出
 * `symbio/src`，同一份代码在两平台生成出不同内容；CI（Linux）的 `--check`
 * 因「Windows 提交的生成物 vs Linux 重生成」逐字比对而必红。
 * 真实文件访问仍走 `path.join`（正斜杠在 Windows 上同样可解析）。
 */
function scopeRow(dir, exts) {
  const impl = walkScope(path.join(ROOT, dir), exts, false);
  const test = walkScope(path.join(ROOT, dir), exts, true);
  // `.rs` 的内联测试模块按归属从「实现」移入「测试」；其它后缀没有这个概念
  const split = exts.includes(".rs")
    ? splitRustLines(impl)
    : { impl: countLines(impl), test: 0 };
  return {
    dir,
    implFiles: impl.length,
    implLines: split.impl,
    testFiles: test.length,
    testLines: countLines(test) + split.test,
  };
}

function scopeRows() {
  return [
    scopeRow("symbio/src", [".rs"]),
    scopeRow("cli/src", [".rs"]),
    scopeRow("tauri/src-tauri/src", [".rs"]),
    scopeRow("tauri/src", [".ts", ".vue"]),
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
    parseVdfsOps(readFileSync(VDFS_PROTOCOL_FILE, "utf8")).length
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
  L.push("> 改了代码不用手动重跑——门禁会**自动重新生成并暂存**（见 `scripts/gate.d/60-facts.mjs`）。");
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
  L.push("> - **配置文件** = 该插件调用过 `announce_configurable`（配置就是 `<根>/<挂载点>/PLUGIN.yml`，");
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
  L.push(
    `- **Tauri IPC**：注册 ${ipc.length} 个 command —— ${fmtList(ipc)}（` +
      "`tauri/src-tauri/src/main.rs::generate_handler!`；`commands.rs` 内另有未注册的" +
      "历史 `#[tauri::command]` 函数，不计入接缝）"
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
