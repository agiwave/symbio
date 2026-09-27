/**
 * line-budget-audit 回归测试
 *
 * 判据的分支双向钉住：
 * 1. 超过基线行数（超标）变红（exit 1）；
 * 2. 低于基线行数（缩减）给出可收紧 WARN 提示（exit 0，--strict 时 exit 1）；
 * 3. 刚好等于基线行数（持平）通过（exit 0）；
 * 4. 测试代码（*.test.rs / #[cfg(test)] 内联模块）不计入行数；
 * 5. 基线中声明了但不存在的目录变红（exit 1）。
 *
 * 跑法：node --test scripts/line-budget-audit.test.mjs
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./line-budget-audit.mjs", import.meta.url));

/**
 * 在临时目录造一棵最小仓库并跑审计。
 * @param {Record<string, string>} files 相对仓库根的路径 → 内容
 * @param {Record<string, { maxLines: number, exts: string[] }>} baselines 基线覆盖
 * @param {string[]} extraArgs 额外 CLI 参数
 */
function audit(files, baselines, extraArgs = []) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "line-budget-audit-"));
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, content);
    }
    const baselineFile = path.join(root, "baselines.json");
    fs.writeFileSync(baselineFile, JSON.stringify(baselines));

    const args = [
      script,
      `--root=${root}`,
      `--baseline-file=${baselineFile}`,
      ...extraArgs,
    ];
    const r = spawnSync(process.execPath, args, {
      env: { ...process.env, NO_COLOR: "1" },
      encoding: "utf8",
      timeout: 20000,
    });
    assert.ifError(r.error);
    return r;
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test("行数超标变红：生产代码行数超过基线（exit 1）", () => {
  const files = {
    "symbio/src/plugins/demo/mod.rs": "fn a() {}\nfn b() {}\nfn c() {}\n",
  };
  const baselines = {
    "symbio/src/plugins/demo": { maxLines: 2, exts: [".rs"] },
  };
  const r = audit(files, baselines);
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.match(r.stderr, /实现行数超标：当前 3 行，超出基线 2 行共 \+1 行/);
});

test("行数持平通过：生产代码行数刚好等于基线（exit 0）", () => {
  const files = {
    "symbio/src/plugins/demo/mod.rs": "fn a() {}\nfn b() {}\n",
  };
  const baselines = {
    "symbio/src/plugins/demo": { maxLines: 2, exts: [".rs"] },
  };
  const r = audit(files, baselines);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /1 项持平，0 项可收紧，0 项超标/);
});

test("行数缩减提示：生产代码低于基线时给出收紧警告（exit 0，--strict 时 exit 1）", () => {
  const files = {
    "symbio/src/plugins/demo/mod.rs": "fn a() {}\n",
  };
  const baselines = {
    "symbio/src/plugins/demo": { maxLines: 3, exts: [".rs"] },
  };
  // 默认模式：exit 0，stdout 输出 WARN
  const r = audit(files, baselines);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /低于基线 3 行（可收紧 -2 行）/);

  // --strict 模式：WARN 视同失败（exit 1）
  const rStrict = audit(files, baselines, ["--strict"]);
  assert.equal(rStrict.status, 1, rStrict.stdout + rStrict.stderr);
  assert.match(rStrict.stdout, /低于基线 3 行（可收紧 -2 行）/);
  assert.match(rStrict.stderr, /行数棘轮审计未通过/);
});

test("测试文件与内联测试不计入生产行数：加测试不触红", () => {
  const files = {
    // 生产代码 2 行，内联测试 5 行
    "symbio/src/plugins/demo/mod.rs": [
      "fn a() {}",
      "fn b() {}",
      "#[cfg(test)]",
      "mod tests {",
      "    #[test]",
      "    fn test_ok() {}",
      "}",
    ].join("\n"),
    // 独立测试文件 10 行
    "symbio/src/plugins/demo/mod.test.rs": [
      "#[test]",
      "fn t1() {}",
      "#[test]",
      "fn t2() {}",
      "",
    ].join("\n"),
  };
  const baselines = {
    "symbio/src/plugins/demo": { maxLines: 2, exts: [".rs"] },
  };
  const r = audit(files, baselines);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /1 项持平/);
});

test("基线范围目录不存在变红：防止基线配置陈旧失效（exit 1）", () => {
  const files = {
    "symbio/src/plugins/real/mod.rs": "fn a() {}\n",
  };
  const baselines = {
    "symbio/src/plugins/nonexistent": { maxLines: 10, exts: [".rs"] },
  };
  const r = audit(files, baselines);
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.match(r.stderr, /目录不存在：symbio\/src\/plugins\/nonexistent/);
});
