/**
 * 事实 · `crates/symbio-core/src/authz/rows.gen.rs` —— 本机授权表的**行**
 *
 * ## 它替掉了什么
 *
 * 「这台机器上有哪些主体、各持有什么能力、内容给谁看」此前是 `authz/mod.rs` 里
 * 一张手写的 `ROWS` 常量：加一个主体 = 改 `core` 的中心文件（D-211 要消灭的形状）。
 * 现在事实源是**各插件自己目录**里的 `PLUGIN.yml`：`principal_*` 行声明
 * 「我是谁、我持有什么、我的内容给谁看」，本生成器把它们投影成那张表。
 *
 * ## 名字仍在代码里，行在清单里
 *
 * `PRINCIPAL_*` 常量留在 `authz/mod.rs`——它们的文档注释解释了**每一行为什么这么窄**
 * （那是设计理由，不是部署事实，塞进 YAML 只会让两份文档各说一半）。本生成器因此
 * 做一件反向核对：**常量集合与清单集合必须相等**。少一条 = 声明了一个没人能引用的
 * 主体；多一条 = 常量指向一行不存在的授权——两种都当场抛错，不生成。
 *
 * `person/local`（用户）是唯一豁免：它**不入能力表**（能力按智能体的权威动作种类划分，
 * 人的发言不是动作种类），所以它有常量而没有行。
 *
 * ## 能力名的闭集从代码里读
 *
 * 生成器不自己维护一份能力名清单——那是 `governance::Capability` 的第二份真相。
 * 它从 `governance/mod.rs` 的 `closed_set!` 声明里抽出拼写，清单里写错一个能力名
 * 当场抛错（而不是等到运行期 `PermissionMatrix::from_names` 拒绝构造，把一条笔误
 * 变成一次「整个授权表降级为空矩阵」的故障）。
 */

import fs from "node:fs";
import path from "node:path";
import { pluginIds, readPluginManifest, pluginManifestRel } from "./_plugin-yml.mjs";

const OUT_REL = "crates/symbio-core/src/authz/rows.gen.rs";
const AUTHZ_MOD_REL = "crates/symbio-core/src/authz/mod.rs";
const GOVERNANCE_REL = "crates/symbio-core/src/governance/mod.rs";

/** 清单键前缀：`principal_main: session/main|reply+…|thread_private`。 */
const KEY_PREFIX = "principal_";

/** 有常量、但**不入表**的主体——理由见 `authz/mod.rs` 的 `PRINCIPAL_USER`。 */
const NOT_A_ROW = new Map([["person/local", "用户发言不是权威动作种类，能力表不为它立行"]]);

const HEADER = `//! 本机主体清单：\`(主体, 能力名, 可见域)\` —— **由 \`scripts/facts/authz-rows.mjs\` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 \`PLUGIN.yml\` 的 \`principal_*\` 行（加一个主体 = 改
//! **自己目录**里的清单；手改本文件会被 \`gen-facts --check\` 判红）。
//!
//! # 这张表是什么
//!
//! 「谁持有什么」与「能不能」是同一张表的两半（模块头的三方边界）：表是**部署事实**
//! ——本机有哪几个主体、各持有什么能力、内容给谁看。能力名的闭集与拼写由
//! \`PermissionMatrix::from_names\` 校验（认不出即拒绝构造），本文件只负责把行摆出来。
//!
//! # 读法
//!
//! 每行三格：主体（可路由地址 \`<namespace>/<name>\`）、能力名列表、可见域。
//! **窄行是唯一入口**（D-099）：未登记的主体在表里没有行 ⇒ 每个谓词为假，
//! 「忘配窄行」的后果是一条**会被看见的拒绝**，不是一次静默超权。
//!
//! 名字与理由住在 \`authz/mod.rs\` 的 \`PRINCIPAL_*\` 常量上（那里写着每行为什么这么窄）；
//! 生成器双向核对两者：常量集合 ≠ 清单集合即抛错。`;

/** 从 `governance/mod.rs` 的 `Capability` 声明里抽拼写——闭集的唯一出处。 */
export function capabilitySpellings(root) {
  const text = fs.readFileSync(path.join(root, ...GOVERNANCE_REL.split("/")), "utf8");
  const start = text.indexOf("pub enum Capability {");
  if (start < 0) throw new Error(`${GOVERNANCE_REL}：找不到 \`pub enum Capability\`——闭集的抽取口径变了`);
  const end = text.indexOf("\n}", start);
  if (end < 0) throw new Error(`${GOVERNANCE_REL}：\`Capability\` 枚举体没有收尾`);
  const body = text.slice(start, end);
  const names = [...body.matchAll(/^\s*([A-Z]\w*)\s*=>\s*"([^"]+)",/gm)].map((m) => m[2]);
  if (names.length === 0) throw new Error(`${GOVERNANCE_REL}：\`Capability\` 里一条拼写都没抽到`);
  return new Set(names);
}

/** `authz/mod.rs` 的 `PRINCIPAL_*` 常量：`主体 id ⇒ 常量名`（值集合由它给出）。 */
export function principalConstants(root) {
  const text = fs.readFileSync(path.join(root, ...AUTHZ_MOD_REL.split("/")), "utf8");
  const out = new Map();
  for (const m of text.matchAll(/^pub const (PRINCIPAL_[A-Z0-9_]+): &str = "([^"]+)";/gm)) {
    const [, name, value] = m;
    const prior = out.get(value);
    if (prior) throw new Error(`${AUTHZ_MOD_REL}：${value} 有两条常量（${prior} / ${name}）——一个主体只能有一个名字`);
    out.set(value, name);
  }
  if (out.size === 0) throw new Error(`${AUTHZ_MOD_REL}：一条 \`PRINCIPAL_*\` 常量都没读到——抽取口径变了`);
  return out;
}

/** 可见域字符串 → `VisScope` 变体名（`thread_private` ⇒ `ThreadPrivate`）。 */
export function scopeVariant(name, where) {
  const map = { thread_private: "ThreadPrivate", shared: "Shared", public: "Public" };
  const v = map[name];
  if (!v) {
    throw new Error(`${where}：可见域 ${JSON.stringify(name)} 不合法（只有 ${Object.keys(map).join(" / ")}）`);
  }
  return v;
}

/**
 * 收集全部声明：`[{ owner, key, id, caps, scope }]`，按插件目录序、再按清单序。
 *
 * 每一条都当场校验：形状、能力名、可见域、主体 id 语法、跨插件重复。
 */
export function collectPrincipals(root) {
  const known = capabilitySpellings(root);
  const consts = principalConstants(root);
  const seen = new Map();
  const rows = [];

  for (const dir of pluginIds(root)) {
    const where = pluginManifestRel(dir);
    const manifest = readPluginManifest(root, dir);
    for (const [key, raw] of Object.entries(manifest)) {
      if (!key.startsWith(KEY_PREFIX)) continue;
      const parts = raw.split("|").map((s) => s.trim());
      if (parts.length !== 3) {
        throw new Error(
          `${where}：\`${key}\` 必须是 \`<主体>|<能力 + 连接>|<可见域>\`（读到 ${parts.length} 段：${JSON.stringify(raw)}）`,
        );
      }
      const [id, capsRaw, scope] = parts;
      if (!/^[a-z][a-z0-9_]*\/[a-z][a-z0-9_-]*$/.test(id)) {
        throw new Error(`${where}：\`${key}\` 的主体 ${JSON.stringify(id)} 不是 \`<namespace>/<name>\` 形态`);
      }
      const caps = capsRaw.split("+").map((s) => s.trim()).filter(Boolean);
      if (caps.length === 0) throw new Error(`${where}：\`${key}\` 一项能力都没声明——空行等于「什么都不许」，请写明意图`);
      for (const cap of caps) {
        if (!known.has(cap)) {
          throw new Error(
            `${where}：\`${key}\` 的能力名 ${JSON.stringify(cap)} 不在 \`governance::Capability\` 的闭集里` +
              `（${[...known].join(" / ")}）——拼错的能力名会让整张表在构造期降级为空矩阵`,
          );
        }
      }
      const variant = scopeVariant(scope, `${where} \`${key}\``);
      const prior = seen.get(id);
      if (prior) throw new Error(`${where}：主体 ${id} 已由 ${prior} 声明——一个主体只能有一行`);
      seen.set(id, where);
      rows.push({ owner: dir, key, id, caps, variant, name: consts.get(id) });
    }
  }

  if (rows.length === 0) {
    throw new Error("没有任何插件声明 `principal_*`——清单读不到，还是声明口径变了？空表 = 所有主体一律拒绝");
  }

  // 双向核对：常量 ⇄ 行。少一条是多出来的常量，多一条是没人能引用的主体。
  for (const id of consts.keys()) {
    if (NOT_A_ROW.has(id) || seen.has(id)) continue;
    throw new Error(
      `\`${AUTHZ_MOD_REL}\` 的常量 ${id} 没有对应的 \`principal_*\` 行——` +
        "声明了主体却没给授权（或是行被删了而常量还在）",
    );
  }
  for (const { id, owner } of rows) {
    if (!consts.has(id)) {
      throw new Error(
        `crates/symbio-plugin-${owner}/PLUGIN.yml 声明的 ${id} 在 \`${AUTHZ_MOD_REL}\` 里没有 \`PRINCIPAL_*\` 常量——` +
          "生成的行谁也引用不到，等于给一个不存在的身份发权限",
      );
    }
  }

  return rows;
}

/** 纯函数：声明清单 → 文件内容。导出给回归测试（测试不写盘）。 */
export function render(rows) {
  const L = [HEADER, ""];
  const names = [...new Set(rows.map((r) => r.name))].sort();
  // `#[rustfmt::skip]`：这一行会随主体增长而过宽，而生成物必须与 rustfmt **逐字同形**
  // ——门禁先 fmt 再生成，若这里让 rustfmt 折行，每次跑完门禁工作区都会多一处格式差异。
  L.push("#[rustfmt::skip]");
  L.push(`use super::{${names.join(", ")}};`);
  L.push("use super::VisScope;");
  L.push("");
  L.push("/// 本机主体清单（生成物；理由住在 `authz/mod.rs` 的常量文档上）。");
  // 一行一行摆（`#[rustfmt::skip]`）：表是**清单的镜像**，一行一个主体才读得出
  // 「这台机器上有谁」；让 rustfmt 按宽度折行会把同一张表折成两种形状。
  L.push("#[rustfmt::skip]");
  L.push("pub(crate) const ROWS: &[(&str, &[&str], VisScope)] = &[");
  let current = null;
  for (const r of rows) {
    if (r.owner !== current) {
      current = r.owner;
      L.push(`    // ============ ${current} ============`);
    }
    const caps = r.caps.map((c) => `"${c}"`).join(", ");
    L.push(`    (${r.name}, &[${caps}], VisScope::${r.variant}),`);
  }
  L.push("];");
  return `${L.join("\n")}\n`;
}

export default {
  id: "authz-rows",
  title: "授权表的行（各插件 `PLUGIN.yml` 的 `principal_*` 投影）",
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => render(collectPrincipals(root)),
};
