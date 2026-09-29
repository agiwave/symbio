import './_selfrun.mjs';
// T19 投影表（B2）：真实会话事实 → 已登记投影按名运行 → 双跑逐字节相同 → 停用即平凡值。
//
// ## 这条用例锁什么
//
// B2 交付两半：**core 的投影域**（单测覆盖签名约束与双跑）与**可选的 `projection` 插件**
// （本用例覆盖）。插件那半有三条性质，只有真跑一整轮才成立：
//
// | 性质 | 判据 | 为什么单测证明不了 |
// |---|---|---|
// | **已登记投影可跨边界运行** | 真实会话派生的事实喂进 `projection/run`，三个会话投影都跑得出结果 | 单测喂手写事实，不是 `fact_log` 从**真实**落盘派生的 |
// | **双跑逐字节相同（A4）** | 同一载荷连调两次，`view` 逐字节相同 | 需真实事实 + 真实插件实例（跨进程） |
// | **平凡值（J2）** | 停用 `projection` ⇒ 两条路由不可达，**投影机制不受影响** | 这是装配方行为，单测里没有装配方 |
//
// ## 为什么先跑一轮真实对话、再经 fact_log 取事实
//
// 投影的输入是 `&[Fact]`（B1 信封）。要证明"投影能吃真实事实"，就不能喂手写 JSON——
// 必须：① 跑真实对话 → 会话落盘；② `fact_log/list` 派生出**真实事实**；③ 把这些事实
// 原样喂进 `projection/run`。于是这条用例同时串起了 B1 与 B2：**事实是投影的原料**。
//
// ## 为什么断言的是"结构投影"而不是消息本体
//
// 投影的输出必须可序列化、可双跑。会话三投影（`session.snapshot` / `display` /
// `checkpoint`）都是**结构投影**——报告"哪些消息会进窗口 / 会被淡化 / 在哪切分"，
// 不回消息正文（事实本就不含原文，它是索引不是副本）。这正是 B1 与 B2 的共同设计。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** gateway 入站配置（与其它用例同款：本机回环、无 token） */
function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
}

/** 解信封：`PluginPayloadWire = { type: 'Data', data: <载荷> }` */
function dataOf(resp, what) {
  assertEq(resp.status, 200, `${what} 应受理（${resp.status}）`);
  const d = resp.body?.data;
  assert(d != null, `${what} 应返回 Data 载荷（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
  return d;
}

/** 三个会话投影的名字（与 `symbio/src/plugins/session/projections.rs` 登记点一致） */
const SESSION_PROJECTIONS = ['session.snapshot', 'session.display', 'session.checkpoint'];

export default defineCase(
  'T19 投影表：已登记投影跨边界运行（双跑一致）· 停用即平凡值',
  async () => {
    // 一轮带工具调用的对话：让落盘既有用户/助手，也有工具节点——事实类型更全
    const llm = await new MockLlm([
      {
        id: 'write-note',
        match: '记一笔',
        toolCalls: [
          {
            id: 'call_note',
            name: 'vdfs_write',
            arguments: { path: 'note.md', text: 'T19 投影表' },
          },
        ],
      },
      { id: 'after-tool', afterTool: true, content: '已记录。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        // `projection` 目录存在 + `plugin_provider` 合格 ⇒ 装配方构造它（可选插件：
        // 不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里，只有目录在才挂载）。
        projection: { enabled: true },
        // fact_log 也装上：本用例用它把真实事实取出来喂给投影
        fact_log: { enabled: true },
      },
    });

    let cli = null;
    try {
      // ① 先跑一轮真实对话 → 会话落盘
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我记一笔',
        provider: PROVIDER_ID,
        session: 'e2e-t19',
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      const msgs = readMessagesJson(hd.homedir, 'e2e-t19');
      assert(Array.isArray(msgs) && msgs.length > 0, '会话应落盘（有消息）');

      // ② 起长驻 CLI（带 gateway）——装配方把 fact_log / projection 都挂上
      cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t19',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      await cli.waitGatewayReady();

      // ③ `projection/list`：三个会话投影**全部**已登记
      const names = dataOf(await cli.invoke('projection/list', {}), 'projection/list');
      assert(Array.isArray(names), `projection/list 应返回数组（实际: ${typeof names}）`);
      for (const n of SESSION_PROJECTIONS) {
        assert(names.includes(n), `应登记投影 ${n}（实得: ${names.join(',')}）`);
      }

      // ④ 用 fact_log 取**真实事实**（B1 → B2 的原料）
      const facts = dataOf(await cli.invoke('fact_log/list', {}), 'fact_log/list');
      assert(Array.isArray(facts) && facts.length > 0, '真实会话应派生出事实');

      // ⑤ 每个会话投影都能用真实事实跑出结果；**双跑逐字节相同（A4）**
      for (const name of SESSION_PROJECTIONS) {
        const payload = { name, facts, at_ms: 1_700_000_000_000 };
        const v1 = dataOf(await cli.invoke('projection/run', payload), `projection/run ${name}`);
        assertEq(v1.name, name, `run 应回显投影名`);
        assert(v1.view != null, `${name} 应返回 view`);
        assert(typeof v1.view.value === 'object', `${name} 的 view.value 应是对象`);
        assertEq(v1.view.trivial, false, `${name} 正常路径不应是平凡值`);

        // 结构投影都应报告 total（本次会话的消息条数）
        assertEq(v1.view.value.total, facts.length, `${name} 的 total 应等于事实条数`);

        const v2 = dataOf(await cli.invoke('projection/run', payload), `projection/run ${name}(第二次)`);
        assertEq(
          JSON.stringify(v2),
          JSON.stringify(v1),
          `${name} 同一载荷两次运行必须逐字节相同（A4）`,
        );
      }

      // ⑥ `session.snapshot` 的结构：小会话（窗口内）应保留全部
      const snap = dataOf(
        await cli.invoke('projection/run', {
          name: 'session.snapshot',
          facts,
          at_ms: 0,
        }),
        'session.snapshot',
      );
      assertEq(snap.view.value.window_turns, 8, '窗口轮数应为 8');
      assertEq(snap.view.value.kept, snap.view.value.total, '小会话应全部保留');

      // ⑦ 未登记的投影名 ⇒ 不可达（可区分"未接入"）
      const missing = await cli.invoke('projection/run', { name: '__no_such__', facts, at_ms: 0 });
      assert(
        missing.status !== 200,
        `未登记投影应不可达（实得 ${missing.status}）`,
      );

      // ⑧ 平凡值（J2）：停用 `projection` ⇒ 两条路由不可达；**其余系统照常**
      cli.stop();
      cli = null;
      const { writeFileSync, readFileSync } = await import('node:fs');
      const { join } = await import('node:path');
      const ymlPath = join(hd.homedir, 'projection', 'PLUGIN.yml');
      writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`);

      const cli2 = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t19-b',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli2.waitGatewayReady();
        for (const path of ['projection/list', 'projection/run']) {
          const disabled = await cli2.invoke(path, { name: 'session.snapshot' });
          assert(
            disabled.status !== 200,
            `停用后 ${path} 应不可达（实得 ${disabled.status}）`,
          );
        }
        // 反证：其余系统照常（gateway + 另一可选插件 fact_log 都还在）
        const root = await cli2.invoke('vdfs/root', {});
        assertEq(root.status, 200, '停用 projection 不应影响 vdfs/root');
        const stillFacts = await cli2.invoke('fact_log/list', {});
        assertEq(stillFacts.status, 200, '停用 projection 不应影响 fact_log');
      } finally {
        cli2.stop();
      }
    } finally {
      cli?.stop();
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
