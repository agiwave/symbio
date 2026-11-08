import './_selfrun.mjs';
// T100 世界模型是一条**插件能力**：一个工具（`world_observe`）落下领域事实；摘掉插件后
// 工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉事实落格；折叠由 owner 的
// `decl.test.rs` 覆盖。
//
// ## 本用例钉的两件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `world_observe` |
// | 事实落格 | 直读会话 WAL 的 `world.observed` 事件有三次观察 |
//
// 三次观察里 `服务器/状态` 被观察两次：两条都留在事实源里，后一条的载荷是
// `离线 / 60`——「后写覆盖」的折叠由 owner 自己验，e2e 只钉这次观察确实落格。
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

const SID = 'e2e-t100';
const SID_OFF = 'e2e-t100-off';

/** 三次观察：`服务器/状态` 被观察两次（后一次是 `离线`），`数据库/版本` 是另一个实体。 */
const OBSERVATIONS = [
  { entity: '服务器', attribute: '状态', value: '在线', confidence: 80 },
  { entity: '服务器', attribute: '状态', value: '离线', confidence: 60 },
  { entity: '数据库', attribute: '版本', value: '16', confidence: 90 },
];

const ASK = '记下这几条观察：服务器先在线、后来离线，数据库版本 16';
const ASK_OFF = '再记一条：磁盘满了';

function scenarios() {
  return [
    {
      id: 'world-observe',
      match: '记下这几条观察',
      times: 1,
      // 一条响应里三次调用：两次同键观察让覆盖在同一轮可见
      toolCalls: OBSERVATIONS.map((o, i) => ({
        id: `call_w${i + 1}`,
        name: 'world_observe',
        arguments: o,
      })),
    },
    { id: 'world-final', afterTool: true, times: 1, content: '这几条观察已记下。' },
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
  'T100 世界模型：工具落领域事实、停用后工具面消失而会话照常',
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
        tools.includes('world_observe'),
        `工具面应含 world_observe（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('这几条观察已记下。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格：直读会话 WAL 里本插件自己的领域事实 ─────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉三次观察原样落格、后一次带 `离线 / 60`。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const observed = events.filter((e) => e.kind === 'world.observed');
      assertEq(
        observed.length,
        3,
        `三次成功观察：${JSON.stringify(observed.map((e) => e.payload))}`,
      );
      assertEq(observed[0]?.payload?.entity, '服务器', '第一条观察实体');
      assertEq(observed[0]?.payload?.attribute, '状态', '第一条观察属性');
      assertEq(observed[0]?.payload?.value, '在线', '第一条观察值');
      assertEq(observed[0]?.payload?.confidence, 80, '第一条观察置信度');
      assertEq(observed[1]?.payload?.value, '离线', '同实体同属性的后一条观察值');
      assertEq(observed[1]?.payload?.confidence, 60, '置信度跟随后一条观察');
      assertEq(observed[2]?.payload?.entity, '数据库', '另一个实体');
      assertEq(observed[2]?.payload?.attribute, '版本', '另一个属性');
      assertEq(observed[2]?.payload?.value, '16', '另一个实体的值');

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      //    「可卸载」的可执行形式是**装配期**不构造（`plugin_enabled: false`），
      //    而不是运行期的某个开关分支。
      const manifest = join(hd.homedir, 'world_model', 'PLUGIN.yml');
      assert(existsSync(manifest), `世界模型插件应已被装配出来（缺 ${manifest}）`);
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
        !offTools.includes('world_observe'),
        `停用后工具面不该再有 world_observe（实际: ${JSON.stringify(offTools)}）`,
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
