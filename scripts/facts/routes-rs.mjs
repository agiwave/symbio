/**
 * 事实 · `crates/symbio-plugin-api/src/routes.rs` —— 跨插件地址常量的**后端侧**投影
 *
 * ## 它替掉了什么
 *
 * 这张常量表此前是**手写的共享文件**：加一条跨插件调用 = 改一个所有插件都依赖、
 * 却不属于任何插件的 Rust 文件。现在它是各插件 `PLUGIN.yml` 里 `plugin_routes:`
 * 那些行的投影——加地址 = 改**自己目录**里的清单，共享文件零 diff（DE-024 D-193）。
 *
 * ## 事实源是清单，注册处仍是 `match` 臂
 *
 * `plugin_routes` 只回答「这个插件导出了哪几条地址给 Rust 调用方」。地址本身是否
 * 真实存在，由插件自己的 `route()` 臂（或 `VDFS_OPS`）回答——生成器逐条核对，
 * 声明了一条没人能到达的地址就直接抛错（**不生成**，不是生成一条幽灵常量）。
 * 前端侧的同名常量由 `routes-ts.mjs` 从同一批臂生成，两栈名字同构。
 *
 * ## 为什么不对全量路由生成常量
 *
 * `routes.gen.ts` 是前后端契约的**全量镜像**；本文件只收**有真实 Rust 调用方**的
 * 地址。给前端专用路由也生成一个后端常量，就是「为将来可能用到预置常量」——
 * 那正是这份表上一轮清理掉的腐烂。插件通过 `plugin_routes` 明确导出范围。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */

import {
  pluginRouteArms,
  routeArmDocs,
  routeConstName,
  vdfsOpIds,
} from "../route-facts.mjs";
import { pluginIds, readPluginManifest } from "./_plugin-yml.mjs";

const OUT_REL = "crates/symbio-plugin-api/src/routes.rs";

const HEADER = `//! 跨插件调用路径常量 —— **由 \`scripts/facts/routes-rs.mjs\` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 \`PLUGIN.yml\` 的 \`plugin_routes:\` 行（相对臂）。
//! 生成器按目录名补前缀、与该插件 \`route()\` 的真实臂（或 \`VDFS_OPS\`）核对后
//! 产出常量。加一条跨插件地址 = 改**自己目录**里的清单；手改本文件会被
//! \`gen-facts --check\` 判红。
//!
//! # 地址规则（展开见 \`docs/003-design/006-插件路由与地址.md\`）
//!
//! ## 一、地址只有两种形态
//!
//! | 形态 | 形状 | 谁写它 | 谁读它 |
//! |---|---|---|---|
//! | **绝对地址** | \`<插件目录名>/<子路径>\` | 调用方（\`ctx.set(PATH, …)\` 后过 \`route\`） | 容器剥首段 → 目标插件的 \`match\` |
//! | **相对臂** | \`<子路径>\` | 目标插件自己的 \`route\` 体内 \`match\` | 容器剥完首段后剩下的那截 |
//!
//! 插件里的 \`match\` 只写相对臂，跨插件调用只写绝对地址——不要在插件内匹配绝对
//! 地址，也不要在调用侧写相对臂。
//!
//! ## 二、前缀是**插件目录名**，不是 \`PluginMeta\`
//!
//! 容器按目录名建实例表并分发（「目录名 = 实例名」），所以目录名才是路由前缀；
//! \`PluginMeta::new\` 的首参不参与路由（E-001 钉住两者相等）。
//!
//! ## 三、过路由才设 \`PATH\`；直连方法**不设**
//!
//! \`PATH\` 是给**容器**看的。判据：下一跳是 \`route()\` 还是方法调用。
//!
//! ## 四、本模块只收**有真实 Rust 调用方**的地址
//!
//! 前端要用的地址是 \`route()\` 臂的生成物（\`tauri/src/constants/routes.gen.ts\`），
//! 不在本模块登记。插件只声明自己要导出给 Rust 调用方的那几条，不为「将来可能
//! 用到」预置常量。
//!
//! # 命名
//!
//! 标识符去掉 \`ROUTE_\` 前缀后，必须**逐字等于**值的大写形式（\`/\` → \`_\`）。
//! 这条与「值必须是真实分发目标」一起由 \`scripts/plugin-entry-audit.mjs\` 的
//! E-012 检查。`;

/** 相对臂的合法形状：生成器只认 match 臂提取器认得的字符，再加一条「不能有 `-`」。 */
const ARM_RE = /^[a-z][a-z0-9_/]*$/;

/** `plugin_routes: a, b` → `["a", "b"]`；空值 / 缺键 ⇒ `[]` */
function parseRouteList(raw, where) {
  if (!raw) return [];
  const arms = raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  const seen = new Set();
  for (const arm of arms) {
    if (!ARM_RE.test(arm) || arm.startsWith("/") || arm.endsWith("/") || arm.includes("//") || arm.split("/").includes("..")) {
      throw new Error(
        `${where}：plugin_routes 里的 ${JSON.stringify(arm)} 不是合法相对臂` +
          "（小写字母 / 数字 / `_` / `/`，不以 `/` 开头或结尾，不含 `..` 与 `-`）",
      );
    }
    if (seen.has(arm)) throw new Error(`${where}：plugin_routes 里的 ${JSON.stringify(arm)} 出现了两次`);
    seen.add(arm);
  }
  return arms;
}

/**
 * 一个插件声明的地址是否真的能被分发。
 *
 * 静态臂与 `VDFS_OPS` 是两种注册形态：前者按目录名补前缀后是绝对地址；后者没有
 * `match` 臂，只能核对「`VDFS_OPS` 是否引用了这条地址的常量名」——按名字核对而不是
 * 解析成值，避免与本次生成互相依赖（首轮生成时 `routes.rs` 还不存在）。
 */
function registrationCheck(root) {
  const vdfsIds = vdfsOpIds(root);
  return (dir, abs) => {
    if (pluginRouteArms(root, dir).includes(abs)) return true;
    return dir === "vdfs" && vdfsIds.has(routeConstName(abs));
  };
}

/** 收集所有声明（绝对地址 + 常量名 + 一句话说明），并对每一条核对其注册处。 */
export function collectDeclaredRoutes(root) {
  const docs = routeArmDocs(root);
  const registered = registrationCheck(root);
  const byRoute = new Map();

  for (const dir of pluginIds(root)) {
    const where = `crates/symbio-plugin-${dir}/PLUGIN.yml`;
    const manifest = readPluginManifest(root, dir);
    for (const arm of parseRouteList(manifest.plugin_routes, where)) {
      const abs = dir === "home" || arm.startsWith(`${dir}/`) ? arm : `${dir}/${arm}`;
      if (!registered(dir, abs)) {
        throw new Error(
          `${where}：plugin_routes 声明了 ${JSON.stringify(arm)}（${abs}），` +
            `但 ${dir} 插件的 route() 臂 / VDFS_OPS 里没有这条地址——生成一条没人能到达的常量比不生成更坏`,
        );
      }
      const prior = byRoute.get(abs);
      if (prior) throw new Error(`${where}：${abs} 已由 ${prior} 声明——同一条地址只能有一个 owner`);
      byRoute.set(abs, dir);
    }
  }

  if (byRoute.size === 0) {
    throw new Error(
      "没有任何插件声明 plugin_routes——清单读不到，还是声明口径变了？空表会让调用方编译失败",
    );
  }

  return [...byRoute.keys()]
    .sort()
    .map((abs) => ({
      route: abs,
      name: routeConstName(abs),
      prefix: abs.split("/")[0],
      // 说明取自注册处（`match` 臂正上方的注释）——vdfs 的三条走 VDFS_OPS，没有臂
      doc: docs.get(abs) ?? (byRoute.get(abs) === "vdfs" ? "经 `VDFS_OPS` 分发（无静态 `match` 臂）" : "注册处：该插件 `route()` 的 `match` 臂"),
    }));
}

/** 纯函数：声明清单 → 文件内容。导出给回归测试（测试不应触发写盘） */
export function render(routes) {
  const L = [HEADER, ""];
  let current = null;
  for (const r of routes) {
    if (r.prefix !== current) {
      current = r.prefix;
      L.push(`// ============ ${current} ============`);
      L.push("");
    }
    L.push(`/// ${r.route} — ${r.doc}`);
    L.push(`pub const ${r.name}: &str = "${r.route}";`);
    L.push("");
  }
  return `${L.join("\n").replace(/\n+$/, "")}\n`;
}

export default {
  id: "routes-rs",
  title: "跨插件地址常量（各插件 `PLUGIN.yml` 的 `plugin_routes` 投影）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => render(collectDeclaredRoutes(root)),
};
