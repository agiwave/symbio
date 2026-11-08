import './_selfrun.mjs';
// T101 可逆性设计是一条**插件能力**：一个工具（`reversibility_annotate`）落下领域事实；
// 摘掉插件后工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉事实落格；折叠由
// owner 的 `decl.test.rs` 覆盖。
//
// ## 本用例钉的两件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `reversibility_annotate` |
// | 事实落格 | 直读会话 WAL 的 `reversibility.assessed` 事件有四次登记 |
//
// 同一动作的两次标注都留在事实源里：`改配置` 先是 `compensable`（带补偿做法），
// 后一条是 `irreversible`——「当前等级」的折叠由 owner 自己验，e2e 只钉四次登记
// 与后一条的载荷。
//
// ## 停用侧断言什么、不断言什么
//
// 运行期停用拿掉的是**插件实例与它注册的工具**；事实声明、视图与行为是**链接期**
// 贡献（D-217 / D-218），它们不会因为翻一个装配位就从二进制里消失。因此这里断言
// 「工具面消失 + 会话照常跑完」，不把链接期贡献的存在与否伪装成运行期读数——
// 与 T88 / T89 / T93 / T100 同一口径。
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

const SID = 'e2e-t101';
const SID_OFF = 'e2e-t101-off';

/** 四次标注：`改配置` 被判两次（后一次更保守），其余各一次。 */
const ASSESSMENTS = [
  { action: '删缓存', level: 'reversible' },
  { action: '发邮件给客户', level: 'irreversible' },
  { action: '改配置', level: 'compensable', undo: '把旧配置写回' },
  { action: '改配置', level: 'irreversible' },
];

const ASK = '标注这几个动作：删缓存、发邮件给客户、改配置';
const ASK_OFF = '再标注一个：删日志';

function scenarios() {
  return [
    {
      id: 'reversibility-annotate',
      match: '标注这几个动作',
      times: 1,
      toolCalls: ASSESSMENTS.map((a, i) => ({
        id: `call_r${i + 1}`,
        name: 'reversibility_annotate',
        arguments: a,
      })),
    },
    { id: 'reversibility-final', afterTool: true, times: 1, content: '这几个动作已标注。' },
    { id: 'off-turn', match: '再标注一个', times: 1, content: '好的。' },
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
  'T101 可逆性设计：工具落可逆等级事实、停用后工具面消失而会话照常',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);

    try {
      // ── A 工具面 ─────────────────────────────────────────────────────────
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
        tools.includes('reversibility_annotate'),
        `工具面应含 reversibility_annotate（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('这几个动作已标注。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格：直读会话 WAL 里本插件自己的领域事实 ─────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉四次登记原样落格、后一条 `改配置` 是 `irreversible`。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const assessed = events.filter((e) => e.kind === 'reversibility.assessed');
      assertEq(
        assessed.length,
        4,
        `四次成功标注都落格（含被修正的那条）：${JSON.stringify(assessed.map((e) => e.payload))}`,
      );
      assertEq(assessed[0]?.payload?.action, ASSESSMENTS[0].action, '第一条动作');
      assertEq(assessed[0]?.payload?.level, ASSESSMENTS[0].level, '第一条等级');
      assertEq(assessed[0]?.payload?.undo, '', '可逆动作没有补偿做法');
      assertEq(assessed[1]?.payload?.action, ASSESSMENTS[1].action, '第二条动作');
      assertEq(assessed[1]?.payload?.level, ASSESSMENTS[1].level, '第二条等级');
      assertEq(assessed[2]?.payload?.action, '改配置', '第三次标注的动作');
      assertEq(assessed[2]?.payload?.level, 'compensable', '第一次判断是可补偿');
      assertEq(assessed[2]?.payload?.undo, '把旧配置写回', '补偿做法原样落格');
      assertEq(assessed[3]?.payload?.action, '改配置', '第四次标注仍是同一动作');
      assertEq(assessed[3]?.payload?.level, 'irreversible', '后一条判断更保守');
      assertEq(assessed[3]?.payload?.undo, '', '不可逆动作没有补偿做法');

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      const manifest = join(hd.homedir, 'reversibility', 'PLUGIN.yml');
      assert(existsSync(manifest), `可逆性插件应已被装配出来（缺 ${manifest}）`);
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
        !offTools.includes('reversibility_annotate'),
        `停用后工具面不该再有 reversibility_annotate（实际: ${JSON.stringify(offTools)}）`,
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
