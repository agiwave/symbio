import './_selfrun.mjs';
// T94 可解释是一条**插件能力**：一个工具（`explain_record`）落下领域事实；摘掉插件后
// 工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉事实落格；折叠由 owner 的
// `decl.test.rs` 覆盖。
//
// ## 本用例钉的是「新增能力只改自己的能力包」
//
// 名册 #49 可解释（`docs/003-design/026-能力拆分与影响类.md` §2.3）单独成包落地。
// 判据必须**同时**看两段，缺一段这条能力就只是「多了一个工具」或「多了一张空表」：
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `explain_record` |
// | 事实落格 | 直读会话 WAL 的 `explain.rationale` 事件有两条 |
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
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';
import { toolNamesOf, envelopeOf, isSilentAuxRequest } from '../llm-contract.mjs';

const SID = 'e2e-t94';
const SID_OFF = 'e2e-t94-off';

/** 两条理由：两条记录，`evidence` 缺省。 */
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
  'T94 可解释：工具落领域事实、停用后工具面消失而会话照常',
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

      // ── B 事实落格：直读会话 WAL 里本插件自己的领域事实 ─────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉「两条记录落格且载荷原样」。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const entries = events.filter((e) => e.kind === 'explain.rationale');
      assertEq(
        entries.length,
        2,
        `两次成功记录 ⇒ 两条理由：${JSON.stringify(entries.map((e) => e.payload))}`,
      );
      assertEq(entries[0]?.payload?.decision, RATIONALES[0].decision, '第一条是记录序里的第一条');
      assertEq(entries[0]?.payload?.why, RATIONALES[0].why, '理由原样落格');
      assertEq(
        entries[0]?.payload?.evidence,
        '',
        `证据缺省为空串（不是 null）：${JSON.stringify(entries[0]?.payload)}`,
      );
      assertEq(entries[1]?.payload?.decision, RATIONALES[1].decision, '第二条接在后面');
      assertEq(entries[1]?.payload?.why, RATIONALES[1].why, '第二条理由原样落格');

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
