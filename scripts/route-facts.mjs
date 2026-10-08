/**
 * route-facts — 「后端有哪些控制面路由」的**唯一提取实现**
 *
 * ## 为什么提取本身要有一份 owner
 *
 * 路由的注册处就是各插件 `async fn route()` 体内的 `match` 臂：容器按**插件目录名**
 * 建实例表、剥掉首段再把余下部分塞回 `PATH` 转发（地址规则见
 * `symbio_core/plugin/route.rs` 的模块文档）。所以「臂 + 目录名」就是那条地址的
 * 完整真相，**没有别的地方登记它**。
 *
 * 三份判定都要读它：`docs/CURRENT.md` §1 的路由列、前端常量
 * `tauri/src/constants/routes.gen.ts`、以及 `plugin-entry-audit`（判某条地址是否真实
 * 存在、报告每条路由的消费方）。各写一遍正则 = 各有一份「什么算一条路由」的判据，
 * 它们会在某个不引人注目的下午分叉（比如一份开始把 `|` 复合臂漏掉）。
 * 本模块因此从 `gen-current-facts.mjs` 与 `plugin-entry-audit.mjs` 里提出这段解析。
 *
 * ## 判据形态（为什么只认 `match` 臂）
 *
 * 只取 `match` 臂左侧的字符串字面量（`"a/b" | "c" => …`），不整段抓字符串——后者会把
 * `get("approved")` 这类参数名当成路由（实测踩坑）。函数体用**大括号配对**取，不按
 * 固定长度截断：截断窗口要么漏掉后面的臂，要么吃进下一个函数。
 *
 * ## 动态分发：臂提不出来，但**词表**在本模块
 *
 * `local/<工具短名>` 按运行期已注册的工具名分发、容器按挂载名分发——静态提不出臂，
 * 因此**不在** `controlPlaneRoutes` 的产出里（`docs/CURRENT.md` §1 把它们标成「动态」）。
 *
 * `vdfs/<操作>` 是第三种形态：它没有 `match` 臂，但操作集合就是源码里那张常量表
 * `VDFS_OPS`，可以读出来（`vdfsOps`）。判「一条地址能不能被分发」时必须把它算进来，
 * 否则 `vdfs/watch` 这类**真实存在**的地址会被读成幽灵——这正是 E-012 要抓的反面。
 */

import { readdirSync, readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { matchBrace, stripComments, stripTestModules } from "./rust-scan.mjs";

/**
 * 「未接线」标记：模块级 `#![allow(dead_code)]`。
 *
 * 整体抑制 dead_code 等于自述「这块代码还没有接线」，它里面的 `CapabilityMeta`
 * 与 `match` 臂都是**未接线的定义**——不排除就会让生成物报出一条模型看不到的工具、
 * 或一条**没人能到达的路由**。
 *
 * 判据取**模块级属性**而不是猜注释文案——可用 grep 复核。当前全仓**无实例**，
 * 保留此判据是为了将来再出现未接线模块时自动生效。
 */
export const UNWIRED_MARKER = "#![allow(dead_code)]";

/** 递归收集插件源码（排除测试文件与 docs 目录：测试里的 meta 不是生产事实） */
export function collectRs(dir) {
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

/** 该插件目录里**参与生产事实**的源码，已去注释、去内联测试模块 */
export function pluginSources(dir) {
  return collectRs(dir)
    .map((f) => readFileSync(f, "utf8"))
    .filter((raw) => !raw.includes(UNWIRED_MARKER))
    .map((raw) => stripTestModules(stripComments(raw)));
}

/**
 * 提取 `async fn <name>(…)` 的函数体（大括号配对）。
 *
 * 只认**第一个** `async fn <name>(`：生产实现总在测试替身之前，且调用方读的是
 * 已剥掉（或抹空）内联测试模块的文本——不过滤的话 `chat_loop/state.test.rs` 里那个
 * `route` 会先被取到，插件就被误判成「零路由臂」。
 */
export function fnBody(txt, name) {
  const i = txt.indexOf(`async fn ${name}(`);
  if (i < 0) return null;
  const open = txt.indexOf("{", i);
  if (open < 0) return null;
  return txt.slice(open, matchBrace(txt, open) + 1);
}

/**
 * 从一个 `route` 函数体里提取**相对臂**（不带插件前缀的 `match` 臂字面量）。
 *
 * 这是「什么算一条路由臂」的唯一语法实现：`extractRouteArms`（生成路由清单与前端
 * 常量）与 `plugin-entry-audit`（判某条地址是否真实存在、报告每条路由的消费方）都读
 * 它。两处各写一遍正则，就会有两份臂的定义——一份开始漏掉 `|` 复合臂时，另一份不会。
 */
export function relativeRouteArms(body) {
  const out = new Set();
  for (const line of body.split("\n")) {
    const eq = line.indexOf("=>");
    if (eq < 0) continue;
    for (const m of line.slice(0, eq).matchAll(/"([a-z][a-z0-9_/-]*)"/g)) {
      if (m[1] === "_") continue;
      out.add(m[1]);
    }
  }
  return [...out].sort();
}

/**
 * 从 `async fn route(…)` 体内提取路由臂，并按插件目录名补成绝对地址。
 *
 * 臂本身是**相对**的：容器剥掉首段再转发，插件只分发余下部分。**`home` 例外**——它是
 * 根容器，收到的 `PATH` 未经剥离，臂本身就是完整地址。
 *
 * ## 前缀取**目录名**，不取 `PluginMeta::new` 的首参
 *
 * 容器按目录名建实例表并在 `route` 里按它分发（`composite.rs`「目录名 = 实例名」），
 * 所以目录名才是真正的路由前缀。`PluginMeta` 首参**不参与路由**——ADR-032 之后它只是
 * 出厂 id；把它当前缀曾产出 `hooks/fire` 这类**不存在的路由**并被下游文档照抄。
 * 改用目录名后「生成器说出的路由」与「容器真正认的路由」同源，而 E-001 另外钉住
 * 「`PluginMeta` 首参 == 目录名」，使两者不会再分叉。
 */
export function extractRouteArms(t, pluginName) {
  const body = fnBody(t, "route");
  if (!body) return [];
  const out = new Set();
  for (const arm of relativeRouteArms(body)) {
    out.add(
      pluginName && pluginName !== "home" && !arm.startsWith(`${pluginName}/`)
        ? `${pluginName}/${arm}`
        : arm
    );
  }
  return [...out].sort();
}

/** 插件根目录（`symbio/src/plugins`）下的一层目录名 = 路由前缀 = 实例名 */
export function pluginDirs(root) {
  const base = path.join(root, "symbio", "src", "plugins");
  return readdirSync(base, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
}

/** 单个插件的静态路由（完整地址，已按目录名补前缀） */
export function pluginRouteArms(root, dirName) {
  const routes = new Set();
  for (const t of pluginSources(path.join(root, "symbio", "src", "plugins", dirName))) {
    for (const arm of extractRouteArms(t, dirName)) routes.add(arm);
  }
  return [...routes].sort();
}

/** 全仓静态可提取的控制面路由（跨插件去重、排序） */
export function controlPlaneRoutes(root) {
  const all = new Set();
  for (const dir of pluginDirs(root)) {
    for (const r of pluginRouteArms(root, dir)) all.add(r);
  }
  return [...all].sort();
}

/**
 * `symbio_core/plugin/route.rs` 里的 `ROUTE_*` 声明（名、值、行号）。
 *
 * 行号来自**去注释后**的文本——`stripComments` 保留行结构，所以报出来的位置就是
 * 编辑器里那一行。注释里的示例（``/// `ROUTE_FOO` ``）因此不会被当成声明。
 */
export function routeConstDecls(root) {
  const abs = path.join(root, "symbio", "src", "symbio_core", "plugin", "route.rs");
  if (!existsSync(abs)) return [];
  const out = [];
  const lines = stripComments(readFileSync(abs, "utf8")).split("\n");
  lines.forEach((line, i) => {
    const m = line.match(
      /(?:pub\s+)?const\s+(ROUTE_[A-Z0-9_]+)\s*:\s*&\s*(?:'static\s+)?str\s*=\s*"([^"]+)"/
    );
    if (m) out.push({ name: m[1], value: m[2], line: i + 1 });
  });
  return out;
}

/**
 * `VDFS_OPS` 里的操作词，解析回字面量（按声明顺序）。
 *
 * 名字有**两种来源**：`VDFS_*` 定义在协议文件里，`ROUTE_VDFS_*` 跨插件可见、归 core。
 * 正则若写成 `/\bVDFS_[A-Z_]+\b/`，`ROUTE_VDFS_ROOT` 里的 `VDFS` 前面是 `_`（词字符）、
 * **没有词边界** ⇒ 匹配不上 ⇒ 三个操作从清单里静默消失，看起来像「操作被删了」而不是
 * 「抽取漏了」。
 *
 * 协议文件不存在 ⇒ 空清单（那说明这个根里没有 vdfs 插件）。对拿它做**成员判定**的
 * 守卫，空集合的失效方向是「什么都判红」，不是静默放行。
 */
export function vdfsOps(root) {
  const abs = path.join(root, "symbio", "src", "plugins", "vdfs", "protocol.rs");
  if (!existsSync(abs)) return [];
  const consts = new Map(routeConstDecls(root).map((d) => [d.name, d.value]));
  const txt = stripComments(readFileSync(abs, "utf8"));
  for (const m of txt.matchAll(/pub const (VDFS_[A-Z0-9_]+)\s*:\s*&\s*(?:'static\s+)?str\s*=\s*"([^"]+)"/g)) {
    consts.set(m[1], m[2]);
  }
  const block = txt.match(/pub const VDFS_OPS\s*:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/);
  if (!block) return [];
  return [...block[1].matchAll(/\b((?:ROUTE_)?VDFS_[A-Z0-9_]+)\b/g)]
    .map((m) => consts.get(m[1]))
    .filter(Boolean);
}

/**
 * 「这条地址能不能被分发」的集合：静态臂 ∪ vdfs 操作词。
 *
 * 它是**充分非必要**的：不在这里不代表不合法（`local/<工具名>`、容器挂载名由运行期
 * 集合决定，静态读不出），所以拿它判红时必须允许逐行豁免并写明运行期来源。反过来，
 * 一张写死的「动态命名空间」白名单会让 `local/serch` 这种拼错的工具名一路放行——
 * 本集合刻意不含白名单。
 */
export function dispatchableRoutes(root) {
  return new Set([...controlPlaneRoutes(root), ...vdfsOps(root)]);
}

/**
 * 地址 → 常量名：`ROUTE_` + 地址大写（`/` → `_`）。
 *
 * 与 `plugin/route.rs` 的命名规则**逐字同构**（「标识符去掉 `ROUTE_` 前缀后必须
 * 等于值的大写形式」），于是后端手写常量与前端生成常量在**名字**上也对得上——
 * 读代码的人不需要在两栈之间做一次翻译。
 */
export function routeConstName(routePath) {
  return `ROUTE_${routePath.replace(/\//g, "_").toUpperCase()}`;
}
