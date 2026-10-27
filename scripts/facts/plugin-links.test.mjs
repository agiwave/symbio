/**
 * facts/plugin-links 回归测试
 *
 * 这两份事实的价值全在「漏一行不报错、只少功能」那一侧：链接行少一条，插件的
 * `inventory` 注册项被链接器丢掉，构建仍然全绿。所以判据必须覆盖三件事——
 * 目录 ⇒ 行的投影正确、标记缺失即抛错、**真仓的生成物与目录一致**。
 *
 * 跑法：node --test scripts/facts/plugin-links.test.mjs
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import cargoFact, { renderInto as renderCargo } from "./plugin-links-cargo.mjs";
import rsFact, { renderInto as renderRs } from "./plugin-links-rs.mjs";
import { pluginLinks, replaceBlock } from "./_plugin-links.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function fixtureRepo(ids) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "plugin-links-"));
  for (const id of ids) {
    const dir = path.join(root, "crates", `symbio-plugin-${id}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, "PLUGIN.yml"), `plugin_id: ${id}\nplugin_assembly: none\n`);
  }
  return root;
}

function withRepo(ids, fn) {
  const root = fixtureRepo(ids);
  try {
    return fn(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

const CARGO_HEAD = "[dependencies]\nsymbio-core = { path = \"../symbio-core\" }\n";
const CARGO_TAIL = "inventory = \"0.3.24\"\n";
const LIB_HEAD = "pub use symbio_core;\n";

test("两个目录 ⇒ 两条依赖行 + 两条链接行，块外字节原样保留", () => {
  withRepo(["alpha", "beta"], (root) => {
    const links = pluginLinks(root);
    assert.deepEqual(
      links.map((l) => l.cargoLine),
      [
        'symbio-plugin-alpha = { path = "../symbio-plugin-alpha" }',
        'symbio-plugin-beta = { path = "../symbio-plugin-beta" }',
      ],
    );
    assert.deepEqual(
      links.map((l) => l.libLine),
      ["pub use symbio_plugin_alpha;", "pub use symbio_plugin_beta;"],
    );

    const cargo = renderCargo(`${CARGO_HEAD}# >>> plugin-links:begin\n# <<< plugin-links:end\n${CARGO_TAIL}`, links);
    assert.ok(cargo.startsWith(CARGO_HEAD), "块前的字节不得被改写");
    assert.ok(cargo.endsWith(CARGO_TAIL), "块后的字节不得被改写");
    assert.ok(cargo.includes('symbio-plugin-beta = { path = "../symbio-plugin-beta" }'));

    const rs = renderRs(`${LIB_HEAD}// >>> plugin-links:begin\n// <<< plugin-links:end\n`, links);
    assert.ok(rs.startsWith(LIB_HEAD));
    assert.ok(rs.includes("pub use symbio_plugin_alpha;"));
  });
});

test("标记缺失 ⇒ 抛错（「什么都没改」的生成器会报绿）", () => {
  withRepo(["alpha"], (root) => {
    const links = pluginLinks(root);
    assert.throws(() => renderCargo(CARGO_HEAD + CARGO_TAIL, links), /标记必须各出现一次/);
    assert.throws(() => renderRs(LIB_HEAD, links), /标记必须各出现一次/);
  });
});

test("标记重复 / 次序反了 ⇒ 抛错", () => {
  const links = [];
  assert.throws(
    () => replaceBlock("# >>> plugin-links:begin\n# >>> plugin-links:begin\n# <<< plugin-links:end\n", {
      begin: "# >>> plugin-links:begin",
      end: "# <<< plugin-links:end",
      lines: [],
      where: "x",
    }),
    /begin×2/,
  );
  assert.throws(
    () => replaceBlock("# <<< plugin-links:end\n# >>> plugin-links:begin\n", {
      begin: "# >>> plugin-links:begin",
      end: "# <<< plugin-links:end",
      lines: [],
      where: "x",
    }),
    /end 标记在 begin 之前/,
  );
  assert.deepEqual(links, []);
});

test("真仓：两份生成物与目录一致（手改过它就红）", () => {
  for (const fact of [cargoFact, rsFact]) {
    const abs = path.join(REPO, ...fact.out.split("/"));
    const text = fs.readFileSync(abs, "utf8");
    assert.equal(fact.render({ root: REPO }), text, `${fact.out} 与目录不一致——重跑 gen-facts`);
  }
});
