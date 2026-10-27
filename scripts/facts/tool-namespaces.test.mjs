/**
 * facts/tool-namespaces 回归测试
 *
 * 这张表的缺省方向是**看得见**的（未声明 ⇒ `legacy/<name>`），所以最坏的失败不是
 * 「漏一个插件」而是「表读空了」——那会让所有工具主体落 `legacy/`，地址还在、
 * 族也对，只是与授权表里的行全部对不上。
 *
 * 跑法：node --test scripts/facts/tool-namespaces.test.mjs
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import fact, { collectNamespaces, render } from "./tool-namespaces.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function withRepo(plugins, fn) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tool-ns-"));
  for (const [id, yml] of Object.entries(plugins)) {
    const dir = path.join(root, "crates", `symbio-plugin-${id}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, "PLUGIN.yml"), yml);
  }
  try {
    return fn(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test("声明了命名空间 ⇒ 进登记表；没声明的插件不贡献条目", () => {
  withRepo(
    {
      vdfs: "plugin_id: vdfs\ntool_namespace: vdfs\n",
      loop: "plugin_id: loop\n",
    },
    (root) => {
      assert.deepEqual(collectNamespaces(root), [{ owner: "vdfs", ns: "vdfs" }]);
      assert.ok(render(collectNamespaces(root)).includes('pub const TOOL_NAMESPACES: &[&str] = &["vdfs"];'));
    },
  );
});

test("一份声明都没有 ⇒ 抛错（空表 = 全部工具主体落 legacy/，与授权表对不上）", () => {
  withRepo({ loop: "plugin_id: loop\n" }, (root) => {
    assert.throws(() => collectNamespaces(root), /没有任何插件声明 `tool_namespace`/);
  });
});

test("命名空间重复 / 形态非法 ⇒ 抛错", () => {
  withRepo(
    {
      vdfs: "plugin_id: vdfs\ntool_namespace: vdfs\n",
      web: "plugin_id: web\ntool_namespace: vdfs\n",
    },
    (root) => {
      assert.throws(() => collectNamespaces(root), /一个命名空间只能有一个 owner/);
    },
  );
  withRepo({ vdfs: "plugin_id: vdfs\ntool_namespace: VDFS/root\n" }, (root) => {
    assert.throws(() => collectNamespaces(root), /不是合法命名空间/);
  });
});

test("真仓：生成物与各插件清单一致（手改过它就红）", () => {
  const abs = path.join(REPO, ...fact.out.split("/"));
  assert.equal(fact.render({ root: REPO }), fs.readFileSync(abs, "utf8"), "重跑 gen-facts");
});
