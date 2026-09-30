import './_selfrun.mjs';
// T18 事实日志（B1）：真实会话落盘 → 可选插件装配 → 事实派生（确定性 + 溯源）→ 停用即平凡值。
//
// ## 这条用例锁什么
//
// B1 交付两半：**core 的事实信封**（单测覆盖）与**可选的 `fact_log` 插件**（本用例覆盖）。
// 插件那半有三条性质，只有真跑一整轮才成立：
//
// | 性质 | 判据 | 为什么单测证明不了 |
// |---|---|---|
// | **从真实存储派生** | 一轮真对话落盘后，`fact_log/list` 能派生出对应事实 | 单测喂的是手写 JSON，不是 session 插件写出来的**真实**形状 |
// | **确定性（A4）** | 同一份磁盘连调两次，两串事实**逐字节相同** | 需要真实存储 + 真实插件实例（跨进程） |
// | **平凡值（J2）** | 停用 `fact_log` ⇒ 该路径不可达，**其余一切照常** | 这是装配方行为，单测里没有装配方 |
//
// ## 为什么走 gateway HTTP 而不是看落盘文件
//
// `fact_log` **不落任何文件**（它只读）。它唯一的可观测面是 `list` 路由——那是
// **机制面**（审计者 / 测试 / 调试者用），不是 LLM 工具。所以本用例在 gateway 边界上
// 直接调 `fact_log/list`，量的是「派生结果」本身，而不是某个渲染器。
//
// ## 溯源（I2）在真实数据上的形态
//
// 一轮对话的落盘是 `user → assistant`（可能还有 `tool_call → tool`）。派生规则：
// - 用户消息 → `turn.user_message`（无前驱）；
// - 助手消息 → `turn.assistant_final` / `.assistant_fallback`，
//   **溯源到同一会话最近的用户消息**；
// - 工具调用 / 结果 → `artifact.added`，溯源到最近的助手消息。
//
// 词形一律是 v2 网格的 `<实体>.<动词>`（`FactKind::wire()` 既是比较用的方法，
// 也是序列化的输出）——**不是** snake_case。
//
// 因此本用例断言：**助手事实的 `caused_by` 恒指向某条更早的用户事实**——
// 这就是 I2「无溯源不声明」在真实链路上的第一次成立。
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

export default defineCase(
  'T18 事实日志：真实会话派生（确定性 + 溯源）· 停用即平凡值',
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
            arguments: { path: 'note.md', text: 'T18 事实日志' },
          },
        ],
      },
      { id: 'after-tool', afterTool: true, content: '已记录。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    // `fact_log` 目录存在 + `plugin_provider` 合格 ⇒ 装配方会构造它（可选插件：不在
    // `ASSEMBLY_SUB_AGENT_PLUGINS` 里，只有目录在才挂载）。
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        fact_log: { enabled: true },
      },
    });

    let cli = null;
    try {
      // ① 先跑一轮真实对话 → 会话落盘（这一步**不经 fact_log**，是纯 session 行为）
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我记一笔',
        provider: PROVIDER_ID,
        session: 'e2e-t18',
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      const msgs = readMessagesJson(hd.homedir, 'e2e-t18');
      assert(Array.isArray(msgs) && msgs.length > 0, '会话应落盘（有消息）');

      // ② 起长驻 CLI（带 gateway）——装配方此刻把 `fact_log` 挂上
      cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t18',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      await cli.waitGatewayReady();

      // ③ 事实派生：`fact_log/list` 返回**只读事实序列**
      const facts1 = dataOf(await cli.invoke('fact_log/list', {}), 'fact_log/list');
      assert(Array.isArray(facts1), `fact_log/list 应返回数组（实际: ${typeof facts1}）`);
      assert(facts1.length > 0, '真实会话应派生出事实（非空）');

      // 形状：六字段齐备（seq / kind / principal / caused_by / at_ms / payload）
      for (const f of facts1) {
        for (const k of ['seq', 'kind', 'principal', 'caused_by', 'at_ms', 'payload']) {
          assert(k in f, `事实应含字段 ${k}（实际键: ${Object.keys(f).join(',')}）`);
        }
      }

      // `seq` 严格递增 —— 回放与溯源的基础（FactSource 契约）
      for (let i = 1; i < facts1.length; i++) {
        assert(
          facts1[i].seq > facts1[i - 1].seq,
          `seq 应严格递增（${facts1[i - 1].seq} → ${facts1[i].seq}）`,
        );
      }

      // principal = 会话 id：事实来自 `e2e-t18` 这一份会话
      assert(
        facts1.every((f) => f.principal === 'e2e-t18'),
        `事实主体应为本会话（实得: ${[...new Set(facts1.map((f) => f.principal))].join(',')}）`,
      );

      // ④ 溯源（I2）：助手事实的 `caused_by` 指向某条更早的用户事实
      const bySeq = new Map(facts1.map((f) => [f.seq, f]));
      // `kind` 的线上词形 = v2 网格的 `<实体>.<动词>`（`FactKind::wire()`），
      // 序列化与 `wire()` 同源——**不是** snake_case。见 `fact/kind.rs` 的说明。
      const assistantFacts = facts1.filter(
        (f) => f.kind === 'turn.assistant_final' || f.kind === 'turn.assistant_fallback',
      );
      assert(assistantFacts.length > 0, '一轮对话应至少产出一条助手事实');
      for (const a of assistantFacts) {
        assert(a.caused_by != null, `助手事实必须带溯源（seq=${a.seq}）`);
        const src = bySeq.get(a.caused_by);
        assert(src, `溯源应指向本序列内的事实（seq=${a.caused_by}）`);
        assertEq(src.kind, 'turn.user_message', `助手事实应溯源到用户消息（实得 ${src.kind}）`);
        assert(a.caused_by < a.seq, '溯源必须指向更早的 seq（派生图无环）');
      }

      // 事实类型：真实一轮含用户消息（+工具节点），至少覆盖用户与助手两类
      const kinds = new Set(facts1.map((f) => f.kind));
      assert(kinds.has('turn.user_message'), `应含用户消息事实（实得: ${[...kinds].join(',')}）`);
      // 反证词形唯一：载荷里不应出现 snake_case 那套写法
      assert(
        ![...kinds].some((k) => k.includes('_') && !k.includes('.')),
        `kind 不应出现 snake_case 词形（实得: ${[...kinds].join(',')}）`,
      );

      // ⑤ 确定性（A4）：同一份磁盘连调两次，两串事实**逐字节相同**
      const facts2 = dataOf(await cli.invoke('fact_log/list', {}), 'fact_log/list(第二次)');
      assertEq(
        JSON.stringify(facts2),
        JSON.stringify(facts1),
        '同一份磁盘两次派生必须逐字节相同（可双跑比对）',
      );

      // ⑥ 平凡值（J2）：停用 `fact_log` ⇒ 该路径不可达；**其余系统照常**（gateway 仍活）
      cli.stop();
      cli = null;
      const { writeFileSync, readFileSync } = await import('node:fs');
      const { join } = await import('node:path');
      const ymlPath = join(hd.homedir, 'fact_log', 'PLUGIN.yml');
      writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`);

      const cli2 = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t18-b',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli2.waitGatewayReady(); // 系统照常起来（停用一个可选插件不影响地基）
        const disabled = await cli2.invoke('fact_log/list', {});
        // 停用 ⇒ 子插件不在位 ⇒ 容器路由 NotFound ⇒ 网关把 Err 映射成 400。
        // （404 是"HTTP 路径没匹配"，与"插件没挂载"是两回事，故这里断言非 200。）
        assert(
          disabled.status !== 200,
          `停用后 fact_log/list 应不可达（实得 ${disabled.status}: ${JSON.stringify(disabled.body)?.slice(0, 200)}）`,
        );
        // 反证：其它路径仍在（不是整个网关挂了）
        const root = await cli2.invoke('vdfs/root', {});
        assertEq(root.status, 200, '停用 fact_log 不应影响 vdfs/root');
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
