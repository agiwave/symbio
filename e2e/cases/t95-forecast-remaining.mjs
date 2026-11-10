import './_selfrun.mjs';
// T95 未来预测：任务事实落格、`forecast.remaining` 不再是对外视图。
//
// ## 本用例钉的是「一条读数能力真的只靠自己目录落地」
//
// 名册 #34「未来预测」（`docs/003-design/026-能力拆分与影响类.md` §2.3）是 `D-294`
// 首条准入能力的落地：`crates/symbio-plugin-forecast` 只折**任务插件的事实**，不新增
// 事实、工具或端口，`kernel` / `sdk` / `loop` / `session` 一行都不动。视图已收回插件
// 私有，e2e 只钉事实落格；折叠由 owner 的 `decl.test.rs` 覆盖。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 折叠读的是**任务事实**而不是预测插件自己编的数 | 直读会话 WAL 的 `task.opened` / `task.asserted` 事件 |
// | `forecast.remaining` 已收回插件私有 | `session/stats` 的 `capability_views` 里没有这个键 |
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';

const SID = 'e2e-t95';
const GW = nextPort();

/** 三条任务：`t2` 等 `t1`；`t3` 已验收完成。 */
const TODOS = [
  { id: 't1', content: '分析架构', status: 'in_progress' },
  { id: 't2', content: '落地实现', status: 'pending', depends_on: ['t1'] },
  { id: 't3', content: '写文档', status: 'completed' },
];

const ASK = '登记这三项任务';

function homedirFor(port) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      // 网关拉起才有 `session/stats` 这条生产路由可打（判据：公开表里没有本视图）
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: GW,
        inbound_token: '',
        inbound_readonly: false,
      },
      ...dialogFaceOff(),
    },
  });
}

export default defineCase(
  'T95 未来预测：任务事实落格、forecast.remaining 不再是对外视图',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'todo-declare',
        match: ASK,
        once: true,
        content: '我先把任务登记进清单。',
        toolCalls: [{ id: 'call_1', name: 'todo_write', arguments: { todos: TODOS } }],
      },
      { id: 'after-todo', afterTool: true, content: '任务清单已登记。' },
    ]).start();
    const hd = homedirFor(llm.port);

    try {
      const turn = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID,
        message: ASK,
      });
      assertEq(turn.code, 0, `CLI 退出码（stderr: ${turn.stderr.slice(0, 600)}）`);
      assert(
        turn.stdout.includes('任务清单已登记'),
        `stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── 事实源：任务插件自己写下的事实（预测插件只读它们，不产事实）────────
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const opened = events.filter((e) => e.kind === 'task.opened');
      assertEq(
        opened.length,
        TODOS.length,
        `三条声明都应开格：${JSON.stringify(opened.map((e) => e.payload))}`,
      );
      assertEq(
        opened.map((e) => e.payload?.task_id),
        ['t1', 't2', 't3'],
        '开格按声明序落格',
      );
      assertEq(opened[0]?.payload?.goal, TODOS[0].content, '第一条任务正文');
      assertEq(opened[1]?.payload?.depends_on, ['t1'], '依赖原样落格');
      assertEq(opened[2]?.payload?.goal, TODOS[2].content, '第三条任务正文');

      const asserted = events.filter((e) => e.kind === 'task.asserted');
      assertEq(
        asserted.length,
        1,
        `只有完成的 t3 被验收：${JSON.stringify(asserted.map((e) => e.payload))}`,
      );
      assertEq(asserted[0]?.payload?.task_id, 't3', '终态的是 t3');

      // ── 部署面：本批「视图收回私有」的判据 ──────────────────────────────
      // 事实在轮末写口落格（D-213），公开表由 `session/stats` 遍历清单生成；
      // 这里只断言 `forecast.remaining` 已不在公开表里，不读它的值。
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GW,
      });
      try {
        await api.waitGatewayReady();
        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);

        const views = inv.body?.data?.capability_views ?? {};
        assert(
          !('forecast.remaining' in views),
          `forecast.remaining 已收回插件私有，不应出现在公开表里（实际键: ${JSON.stringify(Object.keys(views))}）`,
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
