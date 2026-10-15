#!/usr/bin/env node
/**
 * 事实 · 清单归属总账 —— 「一份清单一个 owner」的**可查投影**
 *
 * ## 它补的是哪个洞
 *
 * `lists` 字段（SP-001）+ RG-026 / RG-027 / RG-029 让分工**可机读、可判红**。但那三条
 * 判据只在**出错时**说话：「谁拥有路由清单」这个问题，平时没有出口——要读十份文档的
 * front-matter 才知道。于是「一份清单一个 owner」仍然只活在守卫里，人用不上。
 *
 * 这份总账把它变成**一页可查的表**：每份清单一行，写明 owner 是谁。
 * 它是 front-matter 的**投影**（同 `CURRENT.md` 是代码的投影），所以永远与声明一致。
 *
 * ## 为什么「重复 ⇒ 生成即红」而不是「生成完再判红」
 *
 * 渲染时发现同一个 id 有两个 owner，本模块**直接抛错**：runner 以 exit 1 退出，
 * 一个字都不写。
 *
 * 理由：一份**会说谎的总账**比没有总账更坏——它是给人查的，人会信它。
 * 若渲染出一张「routes 有两个 owner」却让人自己去核对，那这份文档就退化成了
 * 「又一份需要信任的清单」，正是本机制要消灭的东西。**能红的表才配当权威。**
 * （真正的强制力仍在 RG-027；这里是同一件事的第二道，且是**人读**的那一道。）
 *
 * ## 它与 RG-029 的分工
 *
 * - **RG-029** 管「声明与正文标记两面对账」——单文档内，不牵出别的文档；
 * - **本模块** 只读 front-matter 的声明，按 id 归并，渲染出账。标记由 RG-029 守。
 *
 * 两者互不替代：本模块不验证「这份清单真的在正文里」，RG-029 不关心「谁拥有哪份」。
 */

import { fileURLToPath } from "node:url";
import path from "node:path";
import { loadEntries } from "../registry-lib.mjs";

// 本文件住 `scripts/facts/`，仓根上溯两级。
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT_REL = "docs/006-ref/007-清单归属总账.md";
const ENTRY_ID = "RE-007-清单归属总账";
const ENTRY_TITLE = "Symbio 清单归属总账（自动生成）";
const ENTRY_CREATED = "2026-10-14";

/**
 * 从登记装载结果里收 `lists` 声明。
 *
 * `loadEntries` 同时返回**条目**与**投影**（免登记的生成物）：两者都要收——
 * 清单的 owner 很可能就是一份生成物（RE-006 的三张表由 `facts/current` 渲染），
 * 它若隐形，「两个 owner」永远只看得见一个（RG-027 当场漏过一次，见其注释）。
 */
function collect() {
  const { entries, projections } = loadEntries(ROOT);
  const claims = [];
  for (const e of [...entries, ...projections]) {
    const raw = e?.meta?.lists;
    if (!Array.isArray(raw)) continue;
    for (const item of raw) {
      const [id, role] = String(item).split("=").map((s) => s.trim());
      if (id) claims.push({ id, role, rel: e.rel, entryId: e.meta.id ?? null });
    }
  }
  return claims;
}

function render() {
  const claims = collect();

  // ── 分组：id → owners[] ──
  const byId = new Map();
  for (const c of claims) {
    if (!byId.has(c.id)) byId.set(c.id, { owners: [] });
    byId.get(c.id).owners.push(c);
  }

  // ── 重复即红：**渲染前**判，不产出任何文本 ──
  const conflicts = [...byId.entries()].filter(([, v]) => v.owners.length !== 1);
  if (conflicts.length) {
    const detail = conflicts
      .map(([id, v]) => {
        const who = v.owners.length ? v.owners.map((o) => `${o.rel}（${o.entryId ?? "?"}）`).join("、") : "**无 owner**";
        return `  - \`${id}\`：${v.owners.length} 个 owner —— ${who}`;
      })
      .join("\n");
    throw new Error(
      `清单归属有分工重叠，本总账拒绝生成（说谎的表比没有表更坏）：\n${detail}\n` +
        `  处置：留一个 \`owner\`，另一处去掉 \`lists\` 声明、改为在正文指路它（RG-027）。`,
    );
  }

  const ids = [...byId.keys()].sort();
  const L = [];
  L.push("---");
  L.push("kind: ref");
  L.push(`id: ${ENTRY_ID}`);
  L.push(`title: ${ENTRY_TITLE}`);
  L.push("status: active");
  L.push(`created: ${ENTRY_CREATED}`);
  L.push("lists: [lists-index=owner]");
  L.push("---");
  L.push("");
  L.push(`# ${ENTRY_TITLE}`);
  L.push("");
  L.push(
    "> 本表是各文档 front-matter `lists` 字段的**投影**（同 RE-006 是代码的投影）——" +
      "它不新增任何事实，只是把「哪份清单归谁」从「藏在十份文档的 front-matter 里」" +
      "变成一页可查的表。与它同源的判据是 RG-026 / RG-027 / RG-029（字段规范见 " +
      "[`SP-001`](../008-spec/001.md \"SP-001\")）。",
  );
  L.push("");
  L.push("<!-- list:lists-index owner -->");
  L.push("");
  L.push(
    `共 **${ids.length}** 份清单，每份恰好一个 owner（${claims.filter((c) => c.role === "owner").length} 个 owner 声明）。`,
  );
  L.push("");
  L.push("| 清单 id | owner |");
  L.push("|---|---|");
  for (const id of ids) {
    const v = byId.get(id);
    const owner = v.owners[0];
    const ownerCell = owner.entryId
      ? `\`${owner.entryId}\`（\`${owner.rel}\`）`
      : `\`${owner.rel}\``;
    L.push(`| \`${id}\` | ${ownerCell} |`);
  }
  L.push("");
  L.push(
    "> **想读某份清单，去它的 owner 那里读**：role 词表只有 `owner`（`view` 已取消——" +
      "投影重渲是把重复换个形态留下，生成的重复也是重复）。别处要用这些数据就指路，不重渲。",
  );
  L.push("");
  L.push(
    "> **本文档自己也在这张表里**（`lists-index=owner`）——一份关于「清单归属」的文档" +
      "自己也是一份清单。它列自己不是炫技，是自证：投影若连自己都照不出来，说明它不完整。",
  );
  L.push("");
  return L.join("\n");
}

export default {
  id: "lists-index",
  title: "清单归属总账（lists 字段的投影 + 分工重叠即红）",
  out: OUT_REL,
  render,
};
