#!/usr/bin/env node
/**
 * 全仓实现行数棘轮审计（line-budget-audit）
 *
 * 目标：防代码膨胀。对全仓各主要模块/插件的**生产实现行数**设定天花板（基线）：
 * - 超出基线：ERROR 报错，阻断 gate 提交；若增长是有意的，必须显式修改基线并注明日期与理由；
 * - 低于基线：WARN 提示下调收紧棘轮，防止历史债务回弹；
 * - 测试代码（*.test.rs / *.spec.ts / tests.rs / #[cfg(test)] 内联模块）不计入天花板，鼓励写测试。
 *
 * 统计口径与 `scripts/line-count.mjs`（即 `docs/CURRENT.md` §5.1）完全一致。
 *
 * 用法：
 *   node scripts/line-budget-audit.mjs                  # 全仓审计
 *   node scripts/line-budget-audit.mjs --strict         # 告警视同错误（CI 可选）
 *   node scripts/line-budget-audit.mjs ROOT=<dir>       # 指定根目录（测试 fixture 用）
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { red, yellow, green, dim } from "./color.mjs";
import { scopeRow } from "./line-count.mjs";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const defaultRoot = path.resolve(scriptDir, "..");
const rootArg =
  process.argv.find((a) => a.startsWith("ROOT=")) ??
  process.argv.find((a) => a.startsWith("--root="));
const repoRoot = rootArg
  ? path.resolve(rootArg.includes("=") ? rootArg.split("=")[1] : "")
  : defaultRoot;
const baselineArg = process.argv.find((a) => a.startsWith("--baseline-file="));
const STRICT = process.argv.includes("--strict");

/**
 * 生产实现行数天花板基线（按模块细化）
 *
 * 口径：各目录下非测试生产代码的换行数，剥除内联 #[cfg(test)] 模块。
 * 调整基线需显式注明日期与增长理由。
 */
export const LINE_BUDGET_BASELINES = {
  // ── 插件层（symbio/src/plugins/*） ──
  // 2026-09-27：gateway / local / telegram / web 改实现 `PluginConfigMount`（四臂 dispatch
  // 收口到 symbio_core），四处共减 ~137 行；core 因新增该机制 +128 行。
  // 2026-09-27：vdfs 的七个同形工具改由 `tools/spec.rs` 一张表驱动（唯一 Capability
  // 实现），删掉七个逐字复制的工具文件，减 ~75 行（机制行数不变）。
  // 2026-09-27：session 基线由 16802 校正为 16811。原值与实测差 ~9 行：基线是在
  // 更早的 `cargo fmt` 之前记录的，fmt 在 `session` 树内累积了净增。本次用精确实测
  // 而非推算重设（`context/window.rs` 三处硬编码 `24` 提为 `ENTRY_NAME_TOKEN_CAP`，
  // 该文件净 -4 行：16815（未改）→ 16811（改后））。
  "symbio/src/plugins/agent": { maxLines: 3667, exts: [".rs"] },
  "symbio/src/plugins/composite": { maxLines: 1330, exts: [".rs"] },
  "symbio/src/plugins/event_bus": { maxLines: 164, exts: [".rs"] },
  "symbio/src/plugins/gateway": { maxLines: 1190, exts: [".rs"] },
  "symbio/src/plugins/home": { maxLines: 928, exts: [".rs"] },
  "symbio/src/plugins/hook": { maxLines: 467, exts: [".rs"] },
  "symbio/src/plugins/local": { maxLines: 3460, exts: [".rs"] },
  "symbio/src/plugins/mcp": { maxLines: 2897, exts: [".rs"] },
  "symbio/src/plugins/model": { maxLines: 6212, exts: [".rs"] },
  "symbio/src/plugins/plugin_manager": { maxLines: 666, exts: [".rs"] },
  "symbio/src/plugins/session": { maxLines: 16811, exts: [".rs"] },
  "symbio/src/plugins/skill": { maxLines: 1470, exts: [".rs"] },
  "symbio/src/plugins/telegram": { maxLines: 887, exts: [".rs"] },
  "symbio/src/plugins/vdfs": { maxLines: 3012, exts: [".rs"] },
  "symbio/src/plugins/web": { maxLines: 1028, exts: [".rs"] },
  "symbio/src/plugins/work": { maxLines: 627, exts: [".rs"] },

  // ── 内核与驱动层（symbio/src/*） ──
  "symbio/src/symbio_core": { maxLines: 8143, exts: [".rs"] },
  "symbio/src/providers": { maxLines: 2732, exts: [".rs"] },

  // ── 宿主与工具层 ──
  "cli/src": { maxLines: 1573, exts: [".rs"] },
  "tauri/src-tauri/src": { maxLines: 449, exts: [".rs"] },
  "tauri/src": { maxLines: 20269, exts: [".ts", ".vue"] },
};

export function runAudit({ root = repoRoot, baselines = LINE_BUDGET_BASELINES, strict = STRICT } = {}) {
  let errors = 0;
  let warnings = 0;
  const reports = [];

  for (const [scopeDir, cfg] of Object.entries(baselines)) {
    const fullDir = path.join(root, scopeDir);
    if (!fs.existsSync(fullDir)) {
      errors++;
      reports.push({
        type: "error",
        scope: scopeDir,
        msg: `目录不存在：${scopeDir}（基线定义了不存在的范围）`,
      });
      continue;
    }

    const stat = scopeRow(root, scopeDir, cfg.exts);
    const actual = stat.implLines;
    const max = cfg.maxLines;

    if (actual > max) {
      errors++;
      const overflow = actual - max;
      reports.push({
        type: "error",
        scope: scopeDir,
        actual,
        max,
        overflow,
        msg: `${scopeDir} 实现行数超标：当前 ${actual} 行，超出基线 ${max} 行共 +${overflow} 行！\n` +
             `    若增长是有意的，请在 scripts/line-budget-audit.mjs 中显式调大基线并注明日期与理由。`,
      });
    } else if (actual < max) {
      warnings++;
      const slack = max - actual;
      reports.push({
        type: "warn",
        scope: scopeDir,
        actual,
        max,
        slack,
        msg: `${scopeDir} 实现行数已缩减：当前 ${actual} 行，低于基线 ${max} 行（可收紧 -${slack} 行）。`,
      });
    } else {
      reports.push({
        type: "ok",
        scope: scopeDir,
        actual,
        max,
      });
    }
  }

  return { errors, warnings, reports };
}

// CLI 执行模式
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
) {
  console.log(dim(`--- line-budget-audit: 实现行数棘轮审计 ---`));
  let baselines = LINE_BUDGET_BASELINES;
  if (baselineArg) {
    const p = path.resolve(baselineArg.slice("--baseline-file=".length));
    baselines = JSON.parse(fs.readFileSync(p, "utf8"));
  }
  const { errors, warnings, reports } = runAudit({ baselines });

  for (const r of reports) {
    if (r.type === "error") {
      console.error(red(`  ERROR: ${r.msg}`));
    } else if (r.type === "warn") {
      console.log(yellow(`  WARN : ${r.msg}`));
    }
  }

  const okCount = reports.filter((r) => r.type === "ok").length;
  console.log(
    dim(
      `审计完成：共 ${reports.length} 个范围，${okCount} 项持平，${warnings} 项可收紧，${errors} 项超标`
    )
  );

  if (errors > 0 || (STRICT && warnings > 0)) {
    console.error(red(`\n✗ 行数棘轮审计未通过`));
    process.exit(1);
  } else {
    console.log(green(`\n✓ 行数棘轮审计全部通过`));
  }
}
