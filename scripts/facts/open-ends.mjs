/**
 * 事实 · `docs/004-ref/015-未收口项总账.md` —— 「还有多少没收口」的常设读数
 *
 * ## 它替掉了什么
 *
 * `docs/007-task/` 按「收口即删」处置：结论落进设计文档与判定决策，任务文件删除。
 * 于是「欠账归零」与「欠账不可见」在读数上长得一模一样——收口那一刻，唯一能回答
 * 「还有哪些没收口」的载体恰好被删掉。本表补的是这个读数，不是又一本台账：
 * 每一格只投影 owner 的**条数**与**去哪查**，不复制台账内容（一个事实一个 owner）。
 *
 * ## 为什么必须是生成物
 *
 * 手抄的「当前 14 篇」必然与 front-matter 漂移，而漂移没人看得见：复盘时在
 * D-480 撞到手抄的累计欠账读数，实跑 `capability-migration-audit` 已是 0；
 * `docs/README.md` 的 guide 总账也落后于 `registry-board` 的实时读数。本表每格
 * 从 owner 实时取值，`gen-facts --check` 让「读数说谎」变成一次判红。
 *
 * ## 判据不在这里
 *
 * 本表只报数，不判「该不该收口」——那归各 owner 自己的判据（AD / EX / INJ / SRA /
 * RG）。把判定搬进来，会让「改判据」与「改被判物」在同一次提交里互相掩护。
 * 读不出 owner（文件被搬走 / 形状变了）直接抛错，绝不静默报 0：静默的 0
 * 正是「守卫读空了于是全绿」这一类最贵的失效。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */
// @gate lib

import fs from "node:fs";
import path from "node:path";
import { CELLS } from "../../e2e/injection-matrix.mjs";
import { DEBT_EXEMPTIONS } from "../audits/capability-migration-audit.mjs";
import { loadEntries } from "../registry-lib.mjs";
import { walk } from "../fs-walk.mjs";
import { blankComments } from "../rust-scan.mjs";

const OUT_REL = "docs/004-ref/015-未收口项总账.md";

const FRONT_MATTER = `---
kind: ref
id: RE-015-未收口项总账
title: 未收口项总账
status: active
created: 2026-11-13
---`;

/**
 * 一行读数 = 一个 owner 的条数 + 去哪查。
 *
 * `key` 指向 [`collect`] 的返回；新增一类欠账只加一行，并让 `collect` 从它的
 * owner 把数读出来。任何一格都不许写死数字——写死就是下一处 D-480。
 */
export const ROWS = [
  {
    key: "partial",
    unit: "篇",
    label: "design 落地 `partial`",
    owner: "各设计文档 front-matter 的 `impl`",
    how: "`node scripts/registry-board.mjs`",
  },
  {
    key: "archDead",
    unit: "条",
    label: "架构树零生产调用 `pub fn`",
    owner: "`scripts/baselines/arch-dead-code.json`",
    how: "`node scripts/audits/arch-dead-code-audit.mjs --report`",
  },
  {
    key: "sessionFrozen",
    unit: "条",
    label: "session 退役线冻结符号",
    owner: "`scripts/exemptions/plugin-session-sdk.json`",
    how: "`node scripts/audits/session-retire-audit.mjs`",
  },
  {
    key: "debtExemptions",
    unit: "条",
    label: "EX-007 到期豁免",
    owner: "`scripts/audits/capability-migration-audit.mjs`",
    how: "`node scripts/audits/capability-migration-audit.mjs`",
  },
  {
    key: "injectionGaps",
    unit: "格",
    label: "e2e 注入矩阵 `gap`",
    owner: "`e2e/injection-matrix.mjs`",
    how: "`node scripts/audits/injection-matrix-audit.mjs`",
  },
  {
    key: "ignoredTests",
    unit: "处",
    label: "跨 crate 拆分遗留 `#[ignore]`",
    owner: "`crates/` 下的测试文件",
    how: "`grep -rn \"#\\\\[ignore\" crates/ --include=*.test.rs`",
  },
  {
    key: "tasks",
    unit: "条",
    label: "在册任务",
    owner: "`docs/007-task/` 的登记条目",
    how: "`node scripts/registry-board.mjs`",
  },
  {
    key: "verifyCoupled",
    unit: "个",
    label: "verify 程序与生产源码耦合",
    owner: "`docs/003-design/verify/*.rs` 里指向 `crates/` 的 `#[path]` 行",
    how: "`grep -l '#\[path' docs/003-design/verify/*.rs`",
    // 分子分母都由同一份目录现数：16 这个分母写进注释就会漂移（`55-verify.mjs`
    // 的头注释此前正是手抄了 `4/16`）。
    fmt: (v) => `${v.verifyCoupled} / ${v.verifyPrograms}`,
  },
];

/** 一份 JSON 账本的 `symbols.length`；读不出 / 形状变了都抛错，不静默报 0。 */
export function symbolCount(root, rel) {
  const abs = path.join(root, ...rel.split("/"));
  if (!fs.existsSync(abs)) {
    throw new Error(`${OUT_REL}：owner 读不出——${rel} 不存在`);
  }
  let obj;
  try {
    obj = JSON.parse(fs.readFileSync(abs, "utf8"));
  } catch (e) {
    throw new Error(`${OUT_REL}：owner 读不出——${rel} 不是合法 JSON（${e.message}）`);
  }
  if (!Array.isArray(obj?.symbols)) {
    throw new Error(`${OUT_REL}：owner 形状变了——${rel} 缺少 symbols 数组`);
  }
  return obj.symbols.length;
}

/**
 * `#[ignore]` 的处数（跨 crate 拆分后留下的占位用例）。
 *
 * 注释先抹成空白再数，否则「注释里提到这个属性」会被算成一笔欠账；扫描面读空
 * 也抛错——0 个 `.rs` 时返回 0 会让「测试树不见了」和「欠账清零」无法区分。
 */
export function ignoredTests(root) {
  const dir = path.join(root, "crates");
  if (!fs.existsSync(dir)) {
    throw new Error(`${OUT_REL}：owner 读不出——${dir} 不存在`);
  }
  let n = 0;
  let files = 0;
  for (const abs of walk(dir)) {
    if (!abs.endsWith(".rs")) continue;
    files++;
    const text = blankComments(fs.readFileSync(abs, "utf8")).join("\n");
    n += (text.match(/^\s*#\[ignore\b/gm) ?? []).length;
  }
  if (files < 100) {
    throw new Error(`${OUT_REL}：owner 读不出——crates/ 只有 ${files} 份 .rs（< 100）`);
  }
  return n;
}

/**
 * verify 程序的「与生产源码耦合」比：分子 = 含指向 `crates/` 的 `#[path]` 的程序，
 * 分母 = `verify/` 顶层的程序数（`facts/mod.rs` 是生成物，不算一个程序）。
 *
 * 这个比此前只活在 `55-verify.mjs` 的头注释里（手抄 `4/16`）——注释不是 owner，
 * 挂一个新 `#[path]` 就漂。这里从目录与源码现数，判据仍归那个阶段自己。
 */
export function verifyCoupling(root) {
  const dir = path.join(root, "docs", "003-design", "verify");
  if (!fs.existsSync(dir)) {
    throw new Error(`${OUT_REL}：owner 读不出——docs/003-design/verify 不存在`);
  }
  const programs = fs.readdirSync(dir).filter((f) => f.endsWith(".rs"));
  if (programs.length < 10) {
    throw new Error(`${OUT_REL}：owner 读不出——verify/ 只有 ${programs.length} 个程序（< 10）`);
  }
  let coupled = 0;
  for (const f of programs) {
    const src = fs.readFileSync(path.join(dir, f), "utf8");
    // 只认真正指进 `crates/` 的包含；`mod tests` 这类同目录包含不算耦合。
    if(/#\[path\s*=\s*"[^"]*crates\//.test(src)) coupled += 1;
  }
  return { verifyPrograms: programs.length, verifyCoupled: coupled };
}

/** 一次性从各 owner 取数；任何一格读不出就让整份事实生成失败。 */
export function collect(root) {
  const registry = loadEntries(root);
  if (!Array.isArray(registry.entries) || registry.entries.length < 100) {
    throw new Error(
      `${OUT_REL}：登记库只读到 ${registry.entries?.length ?? 0} 条（< 100）——owner 读不出`,
    );
  }
  if (!Array.isArray(CELLS) || CELLS.length < 20) {
    throw new Error(`${OUT_REL}：注入矩阵只读到 ${CELLS?.length ?? 0} 格（< 20）——owner 读不出`);
  }
  if (!Array.isArray(DEBT_EXEMPTIONS)) {
    throw new Error(`${OUT_REL}：DEBT_EXEMPTIONS 不是数组——owner 形状变了`);
  }
  return {
    partial: registry.entries.filter(
      (e) => e.meta?.kind === "design" && e.meta?.impl === "partial",
    ).length,
    // 目录自己的汇总页（`projection: true`）是投影的地址，不是一条任务——
    // 计入会让「在册任务 1 条」与同页「共 0」自相矛盾。
    tasks: registry.entries.filter((e) => e.meta?.kind === "task" && !e.meta?.projection).length,
    archDead: symbolCount(root, "scripts/baselines/arch-dead-code.json"),
    sessionFrozen: symbolCount(root, "scripts/exemptions/plugin-session-sdk.json"),
    debtExemptions: DEBT_EXEMPTIONS.length,
    injectionGaps: CELLS.filter((c) => c?.gap).length,
    ignoredTests: ignoredTests(root),
    ...verifyCoupling(root),
  };
}

/** 纯函数：读数 → 文件内容。导出给回归测试（测试不应触发写盘）。 */
export function render(values) {
  const missing = ROWS.filter((r) => !Number.isInteger(values?.[r.key]));
  if (missing.length) {
    throw new Error(`${OUT_REL}：读数缺项 ${missing.map((r) => r.key).join("、")}`);
  }
  const L = [FRONT_MATTER, "", "# 未收口项总账", ""];
  L.push(
    "> ⚠️ 由 `scripts/facts/open-ends.mjs` 生成，请勿手改。每格数值逐项从 owner 实时读出，",
    "> 手改或 owner 漂移会被 `gen-facts --check` 判红。",
    "",
    "## 1. 这份表回答什么",
    "",
    "`docs/007-task/` 的任务按「收口即删」处置：结论落进设计文档与判定决策，任务文件删除。",
    "于是「还有多少没收口」在任务删掉的那一刻就失去读数——**欠账归零**和**欠账不可见**",
    "长得一模一样。本表不复制任何台账内容，只投影各处 owner 的条数与去哪查：收口要改",
    "的是 owner，本表随 `gen-facts` 自动跟上；owner 没变，读数就不动。",
    "",
    "判据不在这里：本表只报数，不判「该不该收口」。那归各 owner 自己的判据",
    "（AD / EX / INJ / SRA / RG），把判定搬进来会让改判据与改被判物互相掩护。",
    "",
    "## 2. 读数",
    "",
    "| 读数 | 当前 | owner | 怎么查 |",
    "|---|---:|---|---|",
  );
  for (const r of ROWS) {
    const cell = r.fmt ? r.fmt(values) : `${values[r.key]} ${r.unit}`;
    L.push(`| ${r.label} | ${cell} | ${r.owner} | ${r.how} |`);
  }
  L.push(
    "",
    "「当前」不是硬编码：每一格都由 `collect()` 从 owner 现取，owner 读不出",
    "（文件搬走 / 形状变了 / 扫描面空了）直接让生成失败，不静默报 0。",
    "",
    "## 3. 已裁搁置（不计入上表）",
    "",
    "「明确不做」「登记，不修」「留档不做」是**已经裁决过的**，不是等待收口的欠账。",
    "裁决与理由留在各自的 owner，本表不复制：",
    "",
    "- `docs/004-ref/005-e2e覆盖矩阵.md` —— e2e 面的「明确不做 / 暂不补」清单",
    "- `docs/003-design/055-能力自配置终态裁决.md` —— 「登记，不修」的边界",
    "- `docs/003-design/054-判据欠账与到期豁免.md` —— 已裁决搁置与到期豁免的台账",
    "",
    "## 4. 新增一格读数",
    "",
    "在 `scripts/facts/open-ends.mjs` 的 `ROWS` 加一行，并让 `collect()` 从新 owner 把数",
    "读出来（`label / key / unit / owner / how`）。读不出就抛错，不要在别处手抄当前数字——",
    "那正是本表要替掉的漂移。目录即接线，`gen-facts` 会自动把这一格写进下一版。",
    "",
  );
  return L.join("\n");
}

export default {
  id: "open-ends",
  title: "未收口项总账（各处 owner 的条数投影）",
  out: OUT_REL,
  stamp: true,
  render: ({ root }) => render(collect(root)),
};
