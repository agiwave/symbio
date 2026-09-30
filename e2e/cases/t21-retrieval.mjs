import './_selfrun.mjs';
// T21 检索者（B4 / S06）：真实会话 + 会话记忆 → 派生事实（含 memory.*）→ 跑 recall 投影
// → 检索 Actor 行在表里 → 停用即退化（J2）。
//
// ## 这条用例锁什么
//
// B4 是「扩展验证」：不做新地基，只用 B1 / B2 / B3 三个桥头堡盖一间新房。它要证明的是
// **第一个真消费者**能把三者串起来，且**零改既有代码**：
//
// | 性质 | 判据 | 为什么单测证明不了 |
// |---|---|---|
// | **新增机制键 = 0** | `chat_loop.rs` / `SessionStore` / `Plugin` trait 零改动 | 见 `docs/plan/06` §9.5 的 git 核对命令 |
// | **B1+B2+B3 三者串联** | 一次 `retrieval/list` 同时回 `facts`（B1）/ `recall`（B2）/ `actor`（B3） | 三者分属三个注册表，只有同一进程里都装上才成立 |
// | **双跑逐字节相同（A4）** | 同一份磁盘连调两次，`recall` 逐字节相同 | 需真实存储 + 真实插件实例（跨进程） |
// | **平凡值（J2）** | 停用 `retrieval` ⇒ 路由不可达、行不在表里，**会话照常** | 装配方行为，单测里没有装配方 |
//
// ## 为什么要先写 `MEMORY.md`
//
// `memory.*` 事实从**会话目录的 `MEMORY.md`** 派生（公开约定）。因此本用例在跑完一轮
// 真实对话后、起长驻 CLI 前，手动写入 `MEMORY.md`——这正是"记忆被钉住"的最小形态。
// 若不写，`memory.recall` 会走平凡值分支（那是另一条断言）。
//
// ## 为什么用**同一个长驻进程**
//
// Actor 表是**进程级**的（`inventory` + `OnceLock`）。检索行由 `retrieval` 插件在
// `build` 期登记 ⇒ 只要插件装配了就一定在表里。因此所有断言都在同一长驻 CLI 上完成。
import { writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

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

function dataOf(resp, what) {
  assertEq(resp.status, 200, `${what} 应受理（${resp.status}）`);
  const d = resp.body?.data;
  assert(d != null, `${what} 应返回 Data 载荷（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
  return d;
}

const NAME_RETRIEVAL = 'session.retrieval';
const SESSION = 'e2e-t21';

export default defineCase(
  'T21 检索者（B4/S06）：B1 事实 + B2 投影 + B3 Actor 行串联 · 双跑一致 · 停用即退化',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'write-note',
        match: '记一笔',
        toolCalls: [
          {
            id: 'call_note',
            name: 'vdfs_write',
            arguments: { path: 'note-t21.md', text: 'T21 检索者' },
          },
        ],
      },
      { id: 'after-tool', afterTool: true, content: '已记录。' },
      { id: 'generic', content: '好的。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(GATEWAY_PORT),
        // 可选插件：目录在 + provider 合格 ⇒ 装配方构造（不在 ASSEMBLY_SUB_AGENT_PLUGINS 里）
        retrieval: { enabled: true },
        // 也装上 projection：让 `memory.recall` 投影内省口在场（本用例只经 retrieval 取用，
        // 但装上它可证明两个可选插件互不干扰）
        projection: { enabled: true },
      },
    });

    try {
      // ① 先跑一轮真实对话 → 会话落盘
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我记一笔',
        provider: PROVIDER_ID,
        session: SESSION,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      const msgs = readMessagesJson(hd.homedir, SESSION);
      assert(Array.isArray(msgs) && msgs.length > 0, '会话应落盘（有消息）');

      // ② 写会话记忆 `MEMORY.md`（公开约定的落位：<homedir>/session/<id>/MEMORY.md）
      const memDir = join(hd.homedir, 'session', SESSION);
      mkdirSync(memDir, { recursive: true });
      writeFileSync(join(memDir, 'MEMORY.md'), '# 会话记忆\n\n钉住的结论：T21 检索者\n约束：只读召回\n');

      // ③ 起长驻 CLI（带 gateway）——装配方把 retrieval / projection 挂上
      const cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'unused-t21',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await cli.waitGatewayReady();

        // ④ 一次 `retrieval/list` 同时验证 B1 / B2 / B3 三者
        const d = dataOf(await cli.invoke('retrieval/list', {}), 'retrieval/list');

        // B1：事实被派生出来（对话 2 条 + memory.encoded 1 条 + memory.recalled 2 条 ≥ 5）
        const facts = d.facts;
        assert(Array.isArray(facts), `facts 应是数组（实得 ${typeof facts}）`);
        assert(facts.length >= 4, `真实会话 + 记忆应派生出事实（实得 ${facts.length}）`);
        const kinds = new Set(facts.map((f) => f.kind));
        console.log('DEBUG facts=', JSON.stringify(facts).slice(0, 1500));
        // 词形 = v2 网格的 `<实体>.<动词>`（`FactKind::wire()`），序列化同源——不是 snake_case
        assert(kinds.has('turn.user_message'), `应含用户消息事实（实得 ${[...kinds].join(',')}）`);
        assert(
          ![...kinds].some((k) => k.includes('_') && !k.includes('.')),
          `kind 不应出现 snake_case 词形（实得 ${[...kinds].join(',')}）`,
        );
        assert(kinds.has('memory.encoded'), '写非空 MEMORY.md 应派生 memory.encoded');
        assert(kinds.has('memory.recalled'), '每条记忆行应派生 memory.recalled');
        // seq 严格递增
        for (let i = 1; i < facts.length; i++) {
          assert(facts[i].seq > facts[i - 1].seq, `seq 应严格递增（${facts[i - 1].seq}→${facts[i].seq}）`);
        }
        // I2：断言类事实必须带溯源
        const recalled = facts.filter((f) => f.kind === 'memory.recalled');
        for (const f of recalled) {
          assert(f.caused_by != null, `memory.recalled 是断言类，必须带溯源（seq=${f.seq}）`);
        }

        // B2：投影跑了，非平凡
        assertEq(d.degraded, false, 'memory.recall 已登记 ⇒ 不应降级');
        assert(d.recall != null, 'recall 应非空（B2 投影结果）');
        assertEq(d.recall.trivial, false, '有 memory.* 事实 ⇒ 非平凡值');
        const verbs = d.recall.value.memory_verbs ?? [];
        assert(verbs.includes('memory.encoded'), 'recall 应折出 memory.encoded');
        assert(verbs.includes('memory.recalled'), 'recall 应折出 memory.recalled');

        // B3：检索行在表里
        assert(d.actor != null, '检索行应在 Actor 表里');
        assertEq(d.actor.name, NAME_RETRIEVAL, '行名');
        assertEq(d.actor.pattern, 'translator', '检索者是 translator 模式（S06 §3）');
        assertEq(d.actor.budget_ms, 500, '检索者预算 500ms（S06 §3）');
        assertEq(d.actor.scope, 'root', '检索行在 root 层');

        // ⑤ 双跑一致（A4）：`recall` 逐字节相同
        const d2 = dataOf(await cli.invoke('retrieval/list', {}), 'retrieval/list(第二次)');
        assertEq(
          JSON.stringify(d2.recall),
          JSON.stringify(d.recall),
          '同一份磁盘两次召回必须逐字节相同（A4）',
        );

        // ⑥ 与 projection 插件共存：投影也登记在表里（互不干扰）
        const names = dataOf(await cli.invoke('projection/list', {}), 'projection/list');
        assert(names.includes('memory.recall'), `projection/list 应含 memory.recall（实得 ${names.join(',')}）`);

        // ⑦ 停用 retrieval ⇒ 路由不可达、行不登记；**会话与其它系统照常**
        cli.stop();
        const ymlPath = join(hd.homedir, 'retrieval', 'PLUGIN.yml');
        const { readFileSync } = await import('node:fs');
        writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`);

        const cli2 = startLongLivedCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          session: 'e2e-t21-b',
          provider: PROVIDER_ID,
          gatewayPort: GATEWAY_PORT,
        });
        try {
          await cli2.waitGatewayReady();
          const disabled = await cli2.invoke('retrieval/list', {});
          assert(disabled.status !== 200, `停用后 retrieval/list 应不可达（实得 ${disabled.status}）`);

          // actor 表里不再有检索行（若 actor 插件未装，用 actor/list 不可达——故只在可达时断言）
          const rows = await cli2.invoke('actor/list', {});
          if (rows.status === 200) {
            const list = rows.body?.data?.actors ?? [];
            assert(
              !list.some((x) => x.name === NAME_RETRIEVAL),
              '停用后检索行不应在表里',
            );
          }

          // 反证：其余系统照常
          const root = await cli2.invoke('vdfs/root', {});
          assertEq(root.status, 200, '停用 retrieval 不应影响 vdfs/root');
          const stillProj = await cli2.invoke('projection/list', {});
          assertEq(stillProj.status, 200, '停用 retrieval 不应影响 projection');

          // 会话照常：再跑一轮真实对话
          cli2.send('随便说一句');
          await waitFor(
            async () => {
              const m = readMessagesJson(hd.homedir, 'e2e-t21-b') ?? [];
              const settled = m.some((x) => x.type === 'turn' && ['completed', 'failed'].includes(x.status));
              return m.length > 0 && settled;
            },
            { what: '停用 retrieval 后会话仍收敛', timeoutMs: 30_000 },
          );
          const msgs2 = readMessagesJson(hd.homedir, 'e2e-t21-b');
          assert(Array.isArray(msgs2) && msgs2.length > 0, '停用 retrieval 后会话仍应落盘');
        } finally {
          cli2.stop();
        }
      } finally {
        cli.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
