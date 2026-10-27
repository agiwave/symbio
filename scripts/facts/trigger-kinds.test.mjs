/**
 * facts/trigger-kinds 回归测试
 *
 * 登记表读空是这条判据**最坏的失败方向**：空表让所有触发声明在写臂被拒，
 * 功能静默消失而构建全绿。所以「读空了要抛错」是第一条用例。
 *
 * 跑法：node --test scripts/facts/trigger-kinds.test.mjs
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import fact, { collectTriggerKinds, render } from "./trigger-kinds.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function withRepo(plugins, fn) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "trigger-kinds-"));
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

test("一个声明 ⇒ 一个取值，生成物是成员表", () => {
  withRepo({ heartbeat: "plugin_id: heartbeat\ntrigger_kinds: scheduled\n" }, (root) => {
    assert.deepEqual(collectTriggerKinds(root), [{ owner: "heartbeat", kind: "scheduled" }]);
    assert.ok(render(collectTriggerKinds(root)).includes('pub const EVENT_TRIGGER_KINDS: &[&str] = &["scheduled"];'));
  });
});

test("多个取值 ⇒ 按字典序（顺序不影响判据，但生成物必须确定）", () => {
  withRepo({ heartbeat: "plugin_id: heartbeat\ntrigger_kinds: webhook, scheduled\n" }, (root) => {
    assert.ok(render(collectTriggerKinds(root)).includes('&["scheduled", "webhook"]'));
  });
});

test("一份声明都没有 ⇒ 抛错（空表 = 全部触发被拒，功能静默消失）", () => {
  withRepo({ heartbeat: "plugin_id: heartbeat\n" }, (root) => {
    assert.throws(() => collectTriggerKinds(root), /没有任何插件声明 `trigger_kinds`/);
  });
});

test("取值重复 / 形态非法 ⇒ 抛错", () => {
  withRepo(
    {
      heartbeat: "plugin_id: heartbeat\ntrigger_kinds: scheduled\n",
      loop: "plugin_id: loop\ntrigger_kinds: scheduled\n",
    },
    (root) => {
      assert.throws(() => collectTriggerKinds(root), /一个取值只能有一个产出方/);
    },
  );
  withRepo({ heartbeat: "plugin_id: heartbeat\ntrigger_kinds: Scheduled\n" }, (root) => {
    assert.throws(() => collectTriggerKinds(root), /不是合法取值/);
  });
});

test("真仓：生成物与各插件清单一致（手改过它就红）", () => {
  const abs = path.join(REPO, ...fact.out.split("/"));
  assert.equal(fact.render({ root: REPO }), fs.readFileSync(abs, "utf8"), "重跑 gen-facts");
});
