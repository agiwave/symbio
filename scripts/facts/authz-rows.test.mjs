/**
 * facts/authz-rows 回归测试
 *
 * 这张表的失效方式是**静默**的：少一行 = 某个主体什么都做不了（会被看见的拒绝），
 * 多一行 = 给一个不存在的身份发了权限（不会被看见）。所以判据必须覆盖
 * 「清单读不到 / 能力名拼错 / 常量与行对不上 / 真仓生成物漂移」四条。
 *
 * 跑法：node --test scripts/facts/authz-rows.test.mjs
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import fact, { collectPrincipals, render, scopeVariant } from "./authz-rows.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

const GOVERNANCE = `pub enum Capability {
    /// 发言
    Reply => "reply",
    /// 定义工作
    DefineWork => "define.work",
}
`;

const AUTHZ_CONSTS = `pub const PRINCIPAL_MAIN: &str = "session/main";
pub const PRINCIPAL_USER: &str = "person/local";
`;

/**
 * 一棵临时仓库：`plugins` 是 `{ id: yml }`，`consts` 是 `authz/mod.rs` 的常量段，
 * `sdkConsts` 是 `symbio-sdk` 的 `keys/mod.rs` 的常量段。
 *
 * 两处都要建出来：`PRINCIPAL_*` 常量横跨 `keys`（机制层要用的主体名）与 `authz`
 * （授权表自己的主体名）两个文件，生成器读的是**并集**。只建一处，测的就不是真形状。
 */
function fixtureRepo({ plugins, consts = AUTHZ_CONSTS, sdkConsts = '', governance = GOVERNANCE }) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "authz-rows-"));
  for (const [id, yml] of Object.entries(plugins)) {
    const dir = path.join(root, "crates", `symbio-plugin-${id}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, "PLUGIN.yml"), yml);
  }
  const gov = path.join(root, "crates", "symbio-core", "src", "governance");
  fs.mkdirSync(gov, { recursive: true });
  fs.writeFileSync(path.join(gov, "mod.rs"), governance);
  const authz = path.join(root, "crates", "symbio-core", "src", "authz");
  fs.mkdirSync(authz, { recursive: true });
  fs.writeFileSync(path.join(authz, "mod.rs"), consts);
  const keys = path.join(root, "crates", "symbio-sdk", "src", "keys");
  fs.mkdirSync(keys, { recursive: true });
  fs.writeFileSync(path.join(keys, "mod.rs"), sdkConsts);
  return root;
}

function withRepo(opts, fn) {
  const root = fixtureRepo(opts);
  try {
    return fn(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

const OK_PLUGIN = `plugin_id: session
principal_main: session/main|reply+define.work|thread_private
`;

test("一条声明 ⇒ 一行表，用常量名而不是字面量", () => {
  withRepo({ plugins: { session: OK_PLUGIN } }, (root) => {
    const rows = collectPrincipals(root);
    assert.equal(rows.length, 1);
    const text = render(rows);
    assert.ok(text.includes("(PRINCIPAL_MAIN, &[\"reply\", \"define.work\"], VisScope::ThreadPrivate)"));
    assert.ok(text.includes("use super::{PRINCIPAL_MAIN};"));
    assert.ok(text.includes("#[rustfmt::skip]"), "表要一行一个主体（不让 rustfmt 折成两种形状）");
  });
});

test("能力名不在闭集里 ⇒ 抛错（拼错会让整张表在构造期降级为空矩阵）", () => {
  const bad = `plugin_id: session\nprincipal_main: session/main|reply+replay|thread_private\n`;
  withRepo({ plugins: { session: bad } }, (root) => {
    assert.throws(() => collectPrincipals(root), /不在 `governance::Capability` 的闭集里/);
  });
});

test("可见域不认识 ⇒ 抛错", () => {
  assert.throws(() => scopeVariant("everyone", "x"), /可见域/);
  assert.equal(scopeVariant("public", "x"), "Public");
});

test("常量与行对不上（两个方向）都抛错", () => {
  // 多一行：清单声明了主体，`mod.rs` 没有对应常量。
  withRepo({ plugins: { session: OK_PLUGIN } }, (root) => {
    fs.writeFileSync(path.join(root, "crates/symbio-core/src/authz/mod.rs"), "");
    assert.throws(() => collectPrincipals(root), /一条 `PRINCIPAL_\*` 常量都没读到/);
  });

  // 多一条常量：`mod.rs` 有常量，清单里没有行（用户除外）。
  withRepo(
    {
      plugins: { session: `plugin_id: session\n` },
      consts: `pub const PRINCIPAL_MAIN: &str = "session/main";\n`,
    },
    (root) => {
      assert.throws(() => collectPrincipals(root), /没有任何插件声明 `principal_\*`/);
    },
  );

  withRepo(
    {
      plugins: { session: OK_PLUGIN },
      consts: `${AUTHZ_CONSTS}pub const PRINCIPAL_ORPHAN: &str = "session/orphan";\n`,
    },
    (root) => {
      assert.throws(() => collectPrincipals(root), /没有对应的 `principal_\*` 行/);
    },
  );
});

test("同一主体被两个插件声明 ⇒ 抛错（一个主体只能有一行）", () => {
  const dup = `plugin_id: web\nprincipal_main: session/main|reply|thread_private\n`;
  withRepo({ plugins: { session: OK_PLUGIN, web: dup } }, (root) => {
    assert.throws(() => collectPrincipals(root), /一个主体只能有一行/);
  });
});

test("同一主体在两个常量文件里各写一次 ⇒ 抛错（并集仍只许一个名字）", () => {
  // `keys` 与 `authz` 分居两处之后，重复不再表现为「同一文件两行」，
  // 而是「两个文件各一条」——抽取器合并成一张表时若不查重，
  // 后读到的那条会静默覆盖前一条，写错的那个文件永远不会被发现。
  withRepo(
    {
      plugins: { session: OK_PLUGIN },
      consts: `pub const PRINCIPAL_MAIN: &str = "session/main";\n`,
      sdkConsts: `pub const PRINCIPAL_MAIN_COPY: &str = "session/main";\n`,
    },
    (root) => {
      assert.throws(() => collectPrincipals(root), /一个主体只能有一个名字/);
    },
  );
});

test("真仓：生成物与各插件清单一致（手改过它就红）", () => {
  const abs = path.join(REPO, ...fact.out.split("/"));
  assert.equal(fact.render({ root: REPO }), fs.readFileSync(abs, "utf8"), "重跑 gen-facts");
});
