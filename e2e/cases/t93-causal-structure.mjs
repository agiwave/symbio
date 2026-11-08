import './_selfrun.mjs';
// T93 因果结构是一条**插件能力**：一个工具（`causal_assert`）落下领域事实；摘掉插件
// 后工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉事实落格；折叠由 owner 的
// `decl.test.rs` 覆盖。
//
// ## 本用例钉的是「能力包能自己拥有事实链」
//
// 名册 #33 因果结构 + #39 因果表征工具（`docs/003-design/026-能力拆分与影响类.md`
// §2.3）同包落地。它们共用一条事实，因此判据必须**同时**看两段，缺一段这条能力
// 就只是「多了一个工具」或「多了一张空表」：
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `causal_assert` |
// | 事实落格 | 直读会话 WAL 的 `causal.link` 事件，有两条边 |
//
// ## 停用侧断言什么、不断言什么
//
// 运行期停用拿掉的是**插件实例与它注册的工具**；事实声明、视图与行为是**链接期**
// 贡献（D-217 / D-218），它们不会因为翻一个装配位就从二进制里消失。因此这里断言
// 「工具面消失 + 会话照常跑完」，不把链接期贡献的存在与否伪装成运行期读数——
// 与 T88 / T89 同一口径。
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

const SID = 'e2e-t93';
const SID_OFF = 'e2e-t93-off';

/** 两条边组成一条链：`内存不足 → 任务超时 → 重试排队`。 */
const LINKS = [
  { cause: '内存不足', effect: '任务超时', confidence: 70 },
  { cause: '任务超时', effect: '重试排队', confidence: 60 },
];

const ASK = '记下这条因果：内存不足导致任务超时';
const ASK_OFF = '再记一条：磁盘满导致写入失败';

function scenarios() {
  return [
    {
      id: 'causal-tool',
      match: '记下这条因果',
      times: 1,
      // 一条响应里两次调用：链的两段在同一轮落格，视图因此一次折得出整条链
      toolCalls: LINKS.map((l, i) => ({
        id: `call_c${i + 1}`,
        name: 'causal_assert',
        arguments: l,
      })),
    },
    { id: 'causal-final', afterTool: true, times: 1, content: '已记下这条因果。' },
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
  'T93 因果结构：工具落领域事实、停用后工具面消失而会话照常',
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
        tools.includes('causal_assert'),
        `工具面应含 causal_assert（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('已记下这条因果。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格：直读会话 WAL 里本插件自己的领域事实 ─────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉「两次成功断言 ⇒ 两条边落格」。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const links = events.filter((e) => e.kind === 'causal.link');
      assertEq(
        links.length,
        2,
        `两次成功断言 ⇒ 两条边：${JSON.stringify(links.map((e) => e.payload))}`,
      );
      assertEq(links[0]?.payload?.cause, LINKS[0].cause, '第一条边的起点是断言序里的第一条');
      assertEq(links[0]?.payload?.effect, LINKS[0].effect, '第一条边的终点');
      assertEq(
        links[0]?.payload?.confidence,
        LINKS[0].confidence,
        '置信度原样落格（断言强度不换算刻度）',
      );
      assertEq(links[1]?.payload?.cause, LINKS[1].cause, '第二条边接在链的上一段');
      assertEq(links[1]?.payload?.effect, LINKS[1].effect, '第二条边接在链的末端');
      assertEq(links[1]?.payload?.confidence, LINKS[1].confidence, '置信度原样落格');

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      //    「可卸载」的可执行形式是**装配期**不构造（`plugin_enabled: false`），
      //    而不是运行期的某个开关分支。
      const manifest = join(hd.homedir, 'causal', 'PLUGIN.yml');
      assert(existsSync(manifest), `因果插件应已被装配出来（缺 ${manifest}）`);
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
        !offTools.includes('causal_assert'),
        `停用后工具面不该再有 causal_assert（实际: ${JSON.stringify(offTools)}）`,
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
