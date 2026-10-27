/**
 * 事实 · `crates/symbio-kernel/src/event/trigger_kinds.gen.rs` —— **触发方式的登记表**
 *
 * ## 它替掉了什么
 *
 * 「`meta.trigger.kind` 允许取哪些值」此前是 session 写臂里的一句等值比较
 * （`kind != EVENT_TRIGGER_KIND_SCHEDULED`）：加一种触发方式（心跳之外的定时、
 * 事件驱动、外部回调）= 改 `session` 的一条判断——而那正是「加能力要改架构」的残余。
 *
 * 现在事实源是**产出触发的那一方**：`PLUGIN.yml` 的 `trigger_kinds:` 声明「我会写出
 * 哪些 kind」。session 的写臂只判**成员资格**，报错文案直接列出登记过的取值。
 *
 * ## 为什么是登记表而不是放开字符串
 *
 * 不认识的 kind 必须**当场拒**（`VdfsError::invalid`）：静默降级成一条普通用户消息
 * 会让那一轮不再是自主的——事实源里少那四格，而没有任何一处会变红。开放字符串
 * 等于把「拼错」变成「静默换语义」；登记表让新取值成为一个**显式动作**（谁产出谁声明）。
 */

import fs from "node:fs";
import path from "node:path";
import { pluginIds, readPluginManifest, pluginManifestRel } from "./_plugin-yml.mjs";

const OUT_REL = "crates/symbio-kernel/src/event/trigger_kinds.gen.rs";

const HEADER = `//! 触发方式的登记表 —— **由 \`scripts/facts/trigger-kinds.mjs\` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 \`PLUGIN.yml\` 的 \`trigger_kinds:\` 行（谁产出触发，
//! 谁声明它写得出哪些 kind）。消费方只判**成员资格**——加一种触发方式 = 产出方在
//! 自己目录里加一个取值，消费方零 diff。
//!
//! 未登记的 kind 在写臂**当场拒**（不静默降级成普通用户消息）：拼错的触发声明若被
//! 当没看见，那一轮就不再是自主的，而事实源里少的那几格没有任何东西会补。`;

const KIND_RE = /^[a-z][a-z0-9_.]*$/;

/** 收集全部声明：`[{ owner, kind }]`，按插件目录序；重复即抛错。 */
export function collectTriggerKinds(root) {
  const seen = new Map();
  const out = [];
  for (const dir of pluginIds(root)) {
    const where = pluginManifestRel(dir);
    const manifest = readPluginManifest(root, dir);
    const raw = manifest.trigger_kinds;
    if (raw === undefined) continue;
    const kinds = raw.split(",").map((s) => s.trim()).filter(Boolean);
    if (kinds.length === 0) throw new Error(`${where}：\`trigger_kinds\` 是空的——写了就要有取值`);
    for (const kind of kinds) {
      if (!KIND_RE.test(kind)) {
        throw new Error(`${where}：触发方式 ${JSON.stringify(kind)} 不是合法取值（小写字母 / 数字 / \`_\` / \`.\`）`);
      }
      const prior = seen.get(kind);
      if (prior) throw new Error(`${where}：触发方式 ${kind} 已由 ${prior} 声明——一个取值只能有一个产出方`);
      seen.set(kind, where);
      out.push({ owner: dir, kind });
    }
  }
  if (out.length === 0) {
    throw new Error(
      "没有任何插件声明 `trigger_kinds`——清单读不到，还是声明口径变了？" +
        "空登记表会让所有触发声明在写臂被拒（fail-closed，但方向是「功能静默消失」）",
    );
  }
  return out;
}

/** 纯函数：声明清单 → 文件内容。导出给回归测试（测试不写盘）。 */
export function render(decls) {
  const kinds = decls.map((d) => d.kind).sort();
  const L = [HEADER, ""];
  L.push("/// 登记过的触发方式：写体 `meta.trigger.kind` 只认这里面的取值。");
  L.push("#[rustfmt::skip]");
  L.push(`pub const EVENT_TRIGGER_KINDS: &[&str] = &[${kinds.map((k) => `"${k}"`).join(", ")}];`);
  return `${L.join("\n")}\n`;
}

export default {
  id: "trigger-kinds",
  title: "触发方式的登记表（各插件 `PLUGIN.yml` 的 `trigger_kinds` 投影）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => render(collectTriggerKinds(root)),
};
