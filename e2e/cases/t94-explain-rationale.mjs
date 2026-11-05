import './_selfrun.mjs';
// T94 可解释是一条**插件能力**：一个工具（`explain_record`）落下领域事实，一张视图
// （`explain.rationales`）把它们折成可读的理由台账；摘掉插件后工具面消失而系统照常运行。
//
// ## 本用例钉的是「新增能力只改自己的能力包」
//
// 名册 #49 可解释（`docs/003-design/026-能力拆分与影响类.md` §2.3）单独成包落地。
// 判据必须**同时**看三段，缺一段这条能力就只是「多了一个工具」或「多了一张空表」：
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `explain_record` |
// | 事实落格 | 工具调用之后，`session/stats` 的 `capability_views['explain.rationales']` 有两条 |
// | 台账的形状 | 同一格里 `decisions` 按决定去重，`evidence` 缺省为空串 |
//
// ## 为什么第三段判 `decisions` 而不是再判一次 `count`
//
// `count` 只能证明「事实落格了」；`decisions` 是**折出来的**：它要求视图真的把每条
// 记录当成一条台账看过（同一决定只留一次）。少了这一条，一个把每条事实原样吐出来
// 的实现也能让前两段全绿。
//
// ## 为什么经 `session/stats` 读视图
//
// 视图是本插件对自己事实的纯折叠，读它必须经**生产读出口**（`session/stats` 遍历
// 清单生成的公开表，D-221）——只有这条路能证明「装配真的把 owner 的 `ViewDecl`
// 冻结进了同一张表」。直接调用本插件的求值函数是同义反复。
//
// ## 停用侧断言什么、不断言什么
//
// 运行期停用拿掉的是**插件实例与它注册的工具**；事实声明、视图与行为是**链接期**
// 贡献（D-217 / D-218），它们不会因为翻一个装配位就从二进制里消失。因此这里断言
// 「工具面消失 + 会话照常跑完」，不把链接期贡献的存在与否伪装成运行期读数——
// 与 T88 / T89 / T93 同一口径。
import { join } from 'node:path';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
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
import { toolNamesOf, envelopeOf, isSilentAuxRequest } from '../llm-contract.mjs';

const SID = 'e2e-t94';
const SID_OFF = 'e2e-t94-off';
const GW = nextPort();

/** 两条理由：不同决定 ⇒ 台账两格，`decisions` 去重后仍是两条。证据缺省。 */
const RATIONALES = [
  { decision: '切换到增量编译', why: '全量编译超过预算' },
  { decision: '回退到上一版', why: '新版本回归失败' },
];

const ASK = '记下这两条决定：切换到增量编译、回退到上一版';
const ASK_OFF = '再记一条：改用缓存构建';

function scenarios() {
  return [
    {
      id: 'explain-tool',
      match: '记下这两条决定',
      times: 1,
      // 一条响应里两次调用：两格台账在同一轮落格，视图因此一次折得出整张表
      toolCalls: RATIONALES.map((r, i) => ({
        id: `call_e${i + 1}`,
        name: 'explain_record',
        arguments: r,
      })),
    },
    { id: 'explain-final', afterTool: true, times: 1, content: '已记下这两条理由。' },
    // 停用侧：工具已经不存在，模型只能给一句话
    { id: 'off-turn', match: '再记一条', times: 1, content: '好的。' },
  ];
}

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

/** 取第一条主线请求的工具名清单（辅助请求恒不带工具面，`DIALOG_FACE_OFF` 下也没有） */
function firstMainTools(reqs) {
  const main = reqs.find((q) => !isSilentAuxRequest(q) && envelopeOf(q).hasTools);
  return main ? toolNamesOf(main) : null;
}

export default defineCase(
  'T94 可解释：工具落领域事实、视图折出理由台账、停用后工具面消失而会话照常',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);

    try {
      // ── A 工具面：能力包的工具有没有出现在模型看得到的地方 ──────────────
      const turn = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(turn.code, 0, `第一轮 CLI 退出码（stderr: ${turn.stderr.slice(0, 600)}）`);

      const tools = firstMainTools(await llm.requests());
      assert(tools, '应有一条带工具面的主线请求');
      assert(
        tools.includes('explain_record'),
        `工具面应含 explain_record（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('已记下这两条理由。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格 + 视图折出台账：经生产读出口读 owner 的折叠 ───────────
      //
      // 事实在**轮末**写口落格（D-213），因此读数必须在同一进程里读同一份存储——
      // 故这里起一个长驻 CLI 并等网关就绪，而不是再跑一个 one-shot。
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
        const view = views?.['explain.rationales'];
        assert(view, `能力视图列应有 explain.rationales 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(view.status, 'ok', `跨插件读应成功，不是缺席态：${JSON.stringify(view)}`);
        assertEq(view.owner, 'explain', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');
        assertEq(
          view.value?.count,
          2,
          `两次成功记录 ⇒ 两格台账：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.entries?.[0]?.decision,
          RATIONALES[0].decision,
          `第一格是记录序里的第一条：${JSON.stringify(view.value)}`,
        );
        assertEq(view.value?.entries?.[0]?.why, RATIONALES[0].why, '理由原样透出');
        assertEq(
          view.value?.entries?.[0]?.evidence,
          '',
          `证据缺省为空串（不是 null）：${JSON.stringify(view.value)}`,
        );
        assertEq(view.value?.entries?.[1]?.decision, RATIONALES[1].decision, '第二格接在后面');
        assertEq(view.value?.entries?.[1]?.why, RATIONALES[1].why, '第二条理由原样透出');
        // **折出来的**那一格：`decisions` 按决定去重，只留两条不同决定
        assertEq(
          view.value?.decisions,
          [RATIONALES[0].decision, RATIONALES[1].decision],
          `决定应去重且保序：${JSON.stringify(view.value)}`,
        );
      } finally {
        api.stop();
      }

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      //    「可卸载」的可执行形式是**装配期**不构造（`plugin_enabled: false`），
      //    而不是运行期的某个开关分支。
      const manifest = join(hd.homedir, 'explain', 'PLUGIN.yml');
      assert(existsSync(manifest), `可解释插件应已被装配出来（缺 ${manifest}）`);
      const kept = readFileSync(manifest, 'utf8')
        .split('\n')
        .filter((l) => !/^plugin_enabled\s*:/.test(l))
        .join('\n');
      writeFileSync(manifest, `${kept.trimEnd()}\nplugin_enabled: false\n`);
      await llm.reset();

      const off = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_OFF,
        provider: PROVIDER_ID,
        session: SID_OFF,
      });
      assertEq(off.code, 0, `停用后 CLI 应正常退出（stderr: ${off.stderr.slice(0, 600)}）`);

      const offTools = firstMainTools(await llm.requests());
      assert(offTools, '停用后仍应有主线请求（系统照常运行）');
      assert(
        !offTools.includes('explain_record'),
        `停用后工具面不该再有 explain_record（实际: ${JSON.stringify(offTools)}）`,
      );
      assert(
        off.stdout.includes('好的。'),
        `停用后这一轮仍应跑完（实际: ${JSON.stringify(off.stdout)}）`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
