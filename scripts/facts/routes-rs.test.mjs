/**
 * facts/routes-rs 回归测试
 *
 * 生成器的价值全在「声明了但没人能到达」「两个人声明同一条」这些反向用例上：
 * 只测好输入的话，一个把校验全吞掉、照着清单原样输出的生成器也能全绿。
 *
 * 跑法：node --test scripts/facts/routes-rs.test.mjs
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import routesRs, {
  assertRustConsumers,
  collectDeclaredRoutes,
  render,
  rustConsumerCorpus,
} from "./routes-rs.mjs";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const GEN_FILE = path.join(REPO, "crates", "symbio-plugin-api", "src", "routes.rs");

const ROUTE_SRC = `async fn route(self: Arc<Self>, ctx: Arc<dyn PluginInvokeRequest>) -> PluginInvokeResponse<PluginPayload> {
    let path = ctx.get(PATH).unwrap_or_default();
    match path.as_str() {
        // 一条真实臂
        "real" => Ok(PluginPayload::default()),
        "nested/deep" => Ok(PluginPayload::default()),
        _ => Err(PluginError::NotFound("x".into())),
    }
}`;

/**
 * 一棵只有插件目录的临时仓库。条目值整份替换文件；`null` 表示不写。
 */
function fixtureRepo(plugins) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "routes-fact-"));
  for (const [id, files] of Object.entries(plugins)) {
    const dir = path.join(root, "crates", `symbio-plugin-${id}`);
    fs.mkdirSync(path.join(dir, "src"), { recursive: true });
    for (const [name, text] of Object.entries(files)) {
      const abs = path.join(dir, ...name.split("/"));
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      if (text !== null) fs.writeFileSync(abs, text);
    }
  }
  return root;
}

function withRepo(plugins, fn) {
  const root = fixtureRepo(plugins);
  try {
    return fn(root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

const manifest = (extra = "") =>
  `plugin_id: alpha\nplugin_assembly: none\n${extra}`;

test('声明 → 绝对地址 + 常量名；说明取注册处 `match` 臂上方的注释', () => {
  withRepo(
    {
      alpha: {
        "PLUGIN.yml": manifest("plugin_routes: real, nested/deep\n"),
        "src/plugin.rs": ROUTE_SRC,
      },
    },
    (root) => {
      const routes = collectDeclaredRoutes(root);
      assert.deepEqual(
        routes.map((r) => [r.route, r.name]),
        [
          ["alpha/nested/deep", "ROUTE_ALPHA_NESTED_DEEP"],
          ["alpha/real", "ROUTE_ALPHA_REAL"],
        ],
      );
      assert.equal(routes.find((r) => r.route === "alpha/real").doc, "一条真实臂");
      assert.match(routes.find((r) => r.route === "alpha/nested/deep").doc, /注册处/);
    },
  );
});

test('声明了一条 route() 里不存在的地址 ⇒ 抛错（不生成幽灵常量）', () => {
  withRepo(
    {
      alpha: {
        "PLUGIN.yml": manifest("plugin_routes: ghost\n"),
        "src/plugin.rs": ROUTE_SRC,
      },
    },
    (root) => {
      assert.throws(() => collectDeclaredRoutes(root), /没有这条地址/);
    },
  );
});

test('两个插件声明同一条绝对地址 ⇒ 抛错（一个地址只能有一个 owner）', () => {
  // home 是根容器：臂本身就是完整地址，因此它能与 vdfs 撞到同一条绝对地址
  const homeSrc = ROUTE_SRC.replace(/"real"|"nested\/deep"/g, '"vdfs/root"');
  withRepo(
    {
      home: {
        "PLUGIN.yml": "plugin_id: home\nplugin_assembly: none\nplugin_routes: vdfs/root\n",
        "src/plugin.rs": homeSrc,
      },
      vdfs: {
        "PLUGIN.yml": "plugin_id: vdfs\nplugin_assembly: none\nplugin_routes: root\n",
        "src/plugin.rs": "fn unrelated() {}",
        "src/protocol.rs": "pub const VDFS_OPS: &[&str] = &[ROUTE_VDFS_ROOT];",
      },
    },
    (root) => {
      assert.throws(() => collectDeclaredRoutes(root), /已由 home 声明/);
    },
  );
});

test('非法相对臂（含 `-` / 首尾斜杠 / `..`）⇒ 抛错', () => {
  for (const arm of ["weird-arm", "/real", "real/", "../x"]) {
    withRepo(
      {
        alpha: {
          "PLUGIN.yml": manifest(`plugin_routes: ${arm}\n`),
          "src/plugin.rs": ROUTE_SRC,
        },
      },
      (root) => {
        assert.throws(() => collectDeclaredRoutes(root), /不是合法相对臂/, `应拒绝 ${arm}`);
      },
    );
  }
});

test('一条声明都没有 ⇒ 抛错（空表会让调用方编译失败，不是「没有路由」）', () => {
  withRepo(
    {
      alpha: {
        "PLUGIN.yml": manifest(),
        "src/plugin.rs": ROUTE_SRC,
      },
    },
    (root) => {
      assert.throws(() => collectDeclaredRoutes(root), /没有任何插件声明 plugin_routes/);
    },
  );
});

test('render：按首段分组、名↔值同构、文件以单个换行结束', () => {
  const out = render([
    { route: "beta/ping", name: "ROUTE_BETA_PING", prefix: "beta", doc: "乙" },
    { route: "alpha/real", name: "ROUTE_ALPHA_REAL", prefix: "alpha", doc: "甲" },
  ]);
  // render 不排序（排序在 collect 里）；这里只判格式
  assert.match(out, /\/\/ ============ beta ============/);
  assert.match(out, /\/\/ ============ alpha ============/);
  assert.match(out, /\/\/\/ beta\/ping — 乙\npub const ROUTE_BETA_PING: &str = "beta\/ping";/);
  assert.match(out, /\/\/\/ alpha\/real — 甲\npub const ROUTE_ALPHA_REAL: &str = "alpha\/real";/);
  assert.ok(out.endsWith('";\n'));
  assert.ok(!out.endsWith('\n\n'));
});

test('声明了却没有任何真实 Rust 消费方 ⇒ 抛错（头部口径必须机检）', () => {
  const routes = [
    { route: "alpha/real", name: "ROUTE_ALPHA_REAL", prefix: "alpha", doc: "甲" },
  ];
  assert.throws(
    () => assertRustConsumers(routes, ""),
    /没有任何真实 Rust 消费方[\s\S]*ROUTE_ALPHA_REAL[\s\S]*PLUGIN.yml/,
  );
  // 名字整词匹配：`ROUTE_ALPHA_REAL_EXTRA` 不能给 `ROUTE_ALPHA_REAL` 续命
  assert.throws(
    () => assertRustConsumers(routes, "fn f() { let _ = ROUTE_ALPHA_REAL_EXTRA; }"),
    /没有任何真实 Rust 消费方/,
  );
  assert.equal(
    assertRustConsumers(routes, "ctx.set(ROUTE_ALPHA_REAL, payload);"),
    routes,
  );
});

test('消费方语料：注释 / use 转出 / 测试文件 / 生成物自身都不算调用', () => {
  withRepo(
    {
      alpha: {
        "PLUGIN.yml": manifest("plugin_routes: real\n"),
        "src/plugin.rs": ROUTE_SRC,
        "src/caller.rs": [
          "// ROUTE_ALPHA_REAL 只出现在注释里，不算消费方",
          "pub use crate::routes::ROUTE_ALPHA_GHOST;",
          "mod nowhere;",
          "fn unrelated() {}",
        ].join("\n"),
        "src/caller.test.rs": "fn t() { let _ = ROUTE_ALPHA_REAL; }",
      },
      // fixtureRepo 拼出的正是 crates/symbio-plugin-api —— 生成物自身整份排除
      api: {
        "src/routes.rs": 'pub const ROUTE_ALPHA_REAL: &str = "alpha/real";',
      },
    },
    (root) => {
      const corpus = rustConsumerCorpus(root);
      assert.doesNotMatch(corpus, /ROUTE_ALPHA_REAL/);
      assert.doesNotMatch(corpus, /ROUTE_ALPHA_GHOST/);
    },
  );
});

test('真仓：每条 plugin_routes 声明都能在 Rust 生产语料里找到消费方', () => {
  const routes = collectDeclaredRoutes(REPO);
  assert.ok(routes.length > 0);
  assert.equal(assertRustConsumers(routes, rustConsumerCorpus(REPO)), routes);
});

test('真仓的生成物与各插件 PLUGIN.yml 一致（手改过它就红）', () => {
  const committed = fs.existsSync(GEN_FILE) ? fs.readFileSync(GEN_FILE, "utf8") : "";
  assert.equal(committed, routesRs.render({ root: REPO }));
});
