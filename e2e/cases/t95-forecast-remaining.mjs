import './_selfrun.mjs';
// T95 未来预测（读数视图）：任务事实 → `forecast.remaining` → 经生产读出口读回。
//
// ## 本用例钉的是「一条读数能力真的只靠自己目录落地」
//
// 名册 #34「未来预测」（`docs/003-design/026-能力拆分与影响类.md` §2.3）是 `D-294`
// 首条准入能力的落地：`crates/symbio-plugin-forecast` 交**一张视图**，不新增事实、
// 工具或端口，`kernel` / `sdk` / `loop` / `session` 一行都不动。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 视图被装配冻结、可被按名字读 | `session/stats` 的 `capability_views['forecast.remaining']` |
// | 折叠读的是**任务事实**而不是自己编的数 | 声明三条任务、完成一条 ⇒ `{opened: 3, closed: 1, remaining: 2}` |
// | 剩余量是**集合差**不是「开过减断言过的字符串」 | `outstanding` 只剩 `[t1, t2]`（`t3` 已终态） |
//
// ## 为什么经 `session/stats` 读
//
// 视图是 owner 对自己事实的纯折叠；`session/stats` 遍历 `shared_views` 清单生成公开表
// （`D-221`），只有这条路能证明「装配真的把 owner 的 `ViewDecl` 冻结进了同一张表」。
// 直接调用本插件的求值函数是同义反复。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

const SID = 'e2e-t95';
const GW = nextPort();

/** 三条任务：`t2` 等 `t1`；`t3` 已验收完成 ⇒ 剩余 = `t1`、`t2`。 */
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
      // 网关拉起才有 `session/stats` 这条生产路由可打（视图读数走它）
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: GW,
        inbound_token: '',
        inbound_readonly: false,
      },
      session: { ...DIALOG_FACE_OFF },
    },
  });
}

export default defineCase(
  'T95 未来预测：任务事实折成 forecast.remaining，经 session/stats 读回',
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
      const turn = runCli({
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

      // 事实在轮末写口落格（D-213），读数必须在同一进程里读同一份存储。
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

        const views = inv.body?.data?.capability_views;
        const view = views?.['forecast.remaining'];
        assert(
          view,
          `能力视图列应有 forecast.remaining 这一格（实际: ${JSON.stringify(views)}）`,
        );
        assertEq(view.status, 'ok', `跨插件读应成功，不是缺席态：${JSON.stringify(view)}`);
        assertEq(view.owner, 'forecast', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');

        assertEq(view.value?.opened, 3, `三条声明都开过：${JSON.stringify(view.value)}`);
        assertEq(view.value?.closed, 1, `t3 完成了 ⇒ 一条终态：${JSON.stringify(view.value)}`);
        assertEq(view.value?.remaining, 2, `剩 t1、t2 两件没完成：${JSON.stringify(view.value)}`);
        assertEq(
          view.value?.outstanding,
          ['t1', 't2'],
          `剩余量是集合差，已终态的 t3 不在里面：${JSON.stringify(view.value)}`,
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
