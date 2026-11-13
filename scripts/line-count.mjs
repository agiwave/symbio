/**
 * 行数与规模统计口径（唯一真源）
 *
 * 供 `scripts/facts/current.mjs` 等消费方共同引用。
 * 本模块提取自 `facts/current.mjs`，确保事实表口径只有一个来源，绝无漂移。
 *
 * ## 分工：本模块只管「怎么数」，不管「怎么读」
 *
 * 字符串感知的扫描原语（`skipString` / `matchBrace` / `testModuleSpans` /
 * `stripTestModules`）的**唯一真源是 `rust-scan.mjs`**——本文件此前自带一份副本，
 * 是那个模块头注释点名的「四份实现」里唯一没被撤掉的一份（另三份
 * `plugin-entry-audit` / `facts/current` / `core-surface` 早已改指过去）。
 * 两份副本的 `matchBrace` 在未闭合花括号上返回值不同（`length - 1` vs `length`），
 * 正是「同一件东西两个口径」的现成例子，故直接引用，不再保留本地副本。
 */

import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { testModuleSpans } from "./rust-scan.mjs";

/** 目录遍历时跳过的名字（构建产物 / 依赖 / 版本库） */
export const SCOPE_SKIP = ["target", "vendor", "node_modules", "dist", ".git"];

/**
 * 递归收集指定后缀的文件（测试文件按 `*.test.rs` / `*.spec.ts` / `tests.rs` 单独归类，不混进实现）
 */
export function walkScope(dir, exts, isTest) {
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
    const test =
      e.name.endsWith(".test.rs") ||
      e.name.endsWith(".spec.ts") ||
      e.name === "tests.rs";
    if (!exts.some((x) => e.name.endsWith(x)) || test !== isTest) continue;
    out.push(p);
  }
  return out;
}

export function countLines(files) {
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
export function splitRustLines(files) {
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
 * 一个统计范围：`root` 仓库根目录绝对路径，`dir` 相对仓库根，`exts` 参与统计的后缀。
 *
 * ⚠️ `dir` 会**原样进生成物**（§5.1 表格首列），故调用方必须给**正斜杠字面量**，
 * 不得用 `path.join` —— 后者在 Windows 上产出 `symbio\src`、在 Linux 上产出
 * `symbio/src`，同一份代码在两平台生成出不同内容；CI（Linux）的 `--check`
 * 因「Windows 提交的生成物 vs Linux 重生成」逐字比对而必红。
 * 真实文件访问仍走 `path.join`（正斜杠在 Windows 上同样可解析）。
 */
export function scopeRow(root, dir, exts) {
  const impl = walkScope(path.join(root, dir), exts, false);
  const test = walkScope(path.join(root, dir), exts, true);
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
