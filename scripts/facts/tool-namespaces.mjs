/**
 * 事实 · `crates/symbio-kernel/src/chat/tool_namespaces.gen.rs` —— **工具主体的命名空间集合**
 *
 * ## 它替掉了什么
 *
 * `Principal::tool_principal` 里曾写死一段 `"vdfs" | "web" | "local" | "agent"`：
 * 加一个带工具插件的命名空间 = 改 `kernel` 的一处 `match` 臂，而**漏改不会报错**——
 * 那个插件的工具响应主体会静默落成 `legacy/<name>`，地址还在、族也对，只是它
 * 不再与授权表里那一行对齐（`vdfs/write` 那行从此没人引用得到）。
 *
 * 现在事实源是插件自己的 `PLUGIN.yml`：`tool_namespace:` 声明「我的工具主体
 * 住哪个命名空间」。加一个带工具的插件 = 在自己目录里写一行，`kernel` 零 diff。
 *
 * ## 显式声明，不从主体 id 反推
 *
 * 「命名空间是工具族」不是能从 `principal_*` 行推出来的性质：`scheduler/main` 与
 * `session/main` 也是主体，但它们是 System / Agent 族。反推要维护一份「哪些命名空间
 * 不是工具」的黑名单——那又变成一处中心表。声明是显式的（与 D-210 同一条纪律）：
 * 不写 `tool_namespace:` 的插件，其工具一律落 `legacy/<name>`，这是一个
 * **看得见**的缺省（地址里就带着 `legacy`），不是静默失效。
 *
 * `mcp` / `agent` / `legacy` 三个合成命名空间不在这里：它们不是部署事实，
 * 是工具名拼接规则的产物（`mcp__server__tool` 与 `agent_xxx`），住 `principal.rs`。
 */

import fs from "node:fs";
import path from "node:path";
import { pluginIds, readPluginManifest, pluginManifestRel } from "./_plugin-yml.mjs";

const OUT_REL = "crates/symbio-kernel/src/chat/tool_namespaces.gen.rs";

const HEADER = `//! 工具主体的命名空间集合 —— **由 \`scripts/facts/tool-namespaces.mjs\` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 \`PLUGIN.yml\` 的 \`tool_namespace:\` 行。
//! 加一个带工具的插件 = 在**自己目录**里写一行；\`kernel\` 的匹配臂不动。
//!
//! 不在这里的三个命名空间：\`mcp\` / \`agent\` / \`legacy\`——它们是工具名拼接规则的
//! 产物（\`mcp__server__tool\`、\`agent_xxx\`、兜底段），不是部署事实，住 \`principal.rs\`。`;

const NS_RE = /^[a-z][a-z0-9_]*$/;

/** 收集全部声明：`[{ owner, ns }]`，按插件目录序；重复即抛错。 */
export function collectNamespaces(root) {
  const seen = new Map();
  const out = [];
  for (const dir of pluginIds(root)) {
    const where = pluginManifestRel(dir);
    const manifest = readPluginManifest(root, dir);
    const ns = manifest.tool_namespace;
    if (ns === undefined) continue;
    if (!NS_RE.test(ns)) {
      throw new Error(`${where}：\`tool_namespace\` 的 ${JSON.stringify(ns)} 不是合法命名空间（小写字母 / 数字 / \`_\`）`);
    }
    const prior = seen.get(ns);
    if (prior) throw new Error(`${where}：命名空间 ${ns} 已由 ${prior} 声明——一个命名空间只能有一个 owner`);
    seen.set(ns, where);
    out.push({ owner: dir, ns });
  }
  if (out.length === 0) {
    throw new Error(
      "没有任何插件声明 `tool_namespace`——清单读不到，还是声明口径变了？" +
        "空集合会让全部工具主体落 `legacy/<name>`，与授权表里的行对不上",
    );
  }
  return out;
}

/** 纯函数：声明清单 → 文件内容。导出给回归测试（测试不写盘）。 */
export function render(decls) {
  const names = decls.map((d) => d.ns).sort();
  const L = [HEADER, ""];
  L.push("/// 工具主体的命名空间：`<ns>_<tool>` 合成为 `<ns>/<tool>`（授权表按这个地址判）。");
  L.push("#[rustfmt::skip]");
  L.push(`pub const TOOL_NAMESPACES: &[&str] = &[${names.map((n) => `"${n}"`).join(", ")}];`);
  return `${L.join("\n")}\n`;
}

export default {
  id: "tool-namespaces",
  title: "工具主体的命名空间集合（各插件 `PLUGIN.yml` 的 `tool_namespace` 投影）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => render(collectNamespaces(root)),
};
