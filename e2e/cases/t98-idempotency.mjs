import './_selfrun.mjs';
// T98 幂等台账是一条**插件能力**：一个工具（`idempotent_record`）落下领域事实；摘掉
// 插件后工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉事实落格；折叠由
// owner 的 `decl.test.rs` 覆盖。
//
// ## 本用例钉的是「新增能力只改自己的能力包」
//
// 名册 #37 幂等与重试（`docs/003-design/026-能力拆分与影响类.md` §2.3）的**幂等半边**
// 单独成包落地；重试不在本片（重放身份与续写幕的口径属后续片）。判据必须**同时**看四段：
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `idempotent_record` |
// | 事实落格 | 同一键第二次调用在执行前被拒，直读会话 WAL 的 `idempotency.recorded` 仍只有一条 |
// | 普通工具不受影响 | 装与不装本插件，`notes_add` 两次不同正文都落两条 `notes.created` |
//
// ## 为什么跨两轮调同一个键
//
// 同轮同键由写口按事件 id 去重；**执行前拒绝**只有跨轮才看得见——第一轮的
// `idempotency.recorded` 已在会话事实里，第二轮 `before` 读到它才拒。只判同轮会漏掉
// 这条判据的一半。
//
// ## 停用侧断言什么、不断言什么
//
// 运行期停用拿掉的是**插件实例与它注册的工具**；事实声明、视图与行为是**链接期**
// 贡献（D-217 / D-218），它们不会因为翻一个装配位就从二进制里消失。因此这里断言
// 「工具面消失 + 普通工具照常 + 会话照常跑完」，不把链接期贡献的存在与否伪装成
// 运行期读数——与 T88 / T94 同一口径。
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

const SID = 'e2e-t98';
const SID_OFF = 'e2e-t98-off';

/** 同一个键两次：第一次落格，第二次在执行前被拒。 */
const KEY = 'build-42';
const VALUE_FIRST = 'compiled';
const VALUE_SECOND = 'again';

/** 普通工具（便签）的两条不同正文——装与不装都该落两格。 */
const NOTES = ['alpha', 'beta'];

const ASK_FIRST = '把 build-42 记成 compiled';
const ASK_DUP = '再记一次 build-42';
const ASK_NOTES = '记两条便签';
const ASK_OFF_NOTES = '离线记两条便签';

function noteCalls(prefix) {
  return NOTES.map((text, i) => ({
    id: `${prefix}${i + 1}`,
    name: 'notes_add',
    arguments: { text },
  }));
}

function scenarios() {
  return [
    {
      id: 'idem-first',
      match: '把 build-42',
      times: 1,
      toolCalls: [{ id: 'call_i1', name: 'idempotent_record', arguments: { key: KEY, value: VALUE_FIRST } }],
    },
    { id: 'idem-first-final', match: '把 build-42', afterTool: true, times: 1, content: '已记下。' },
    {
      id: 'idem-dup',
      match: '再记一次',
      times: 1,
      toolCalls: [{ id: 'call_i2', name: 'idempotent_record', arguments: { key: KEY, value: VALUE_SECOND } }],
    },
    { id: 'idem-dup-final', match: '再记一次', afterTool: true, times: 1, content: '这个键已经记过。' },
    { id: 'notes-on', match: '记两条便签', times: 1, toolCalls: noteCalls('call_n') },
    { id: 'notes-on-final', match: '记两条便签', afterTool: true, times: 1, content: '好了。' },
    // 停用侧：幂等工具不存在，只有普通便签
    { id: 'notes-off', match: '离线记两条便签', times: 1, toolCalls: noteCalls('call_o') },
    { id: 'notes-off-final', match: '离线记两条便签', afterTool: true, times: 1, content: '好了。' },
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
  'T98 幂等台账：同键跨轮被拒、装与不装普通工具行为一致',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);

    try {
      // ── A 工具面：能力包的工具有没有出现在模型看得到的地方 ──────────────
      const first = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_FIRST,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 600)}）`);

      const tools = firstMainTools(await llm.requests());
      assert(tools, '应有一条带工具面的主线请求');
      assert(
        tools.includes('idempotent_record'),
        `工具面应含 idempotent_record（实际: ${JSON.stringify(tools)}）`,
      );
      assert(tools.includes('notes_add'), '普通工具（notes_add）应同时在场');
      assert(first.stdout.includes('已记下。'), `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(first.stdout)}）`);

      // ── B 跨轮同键：第二次在执行前被拒，台账仍只有一格 ──────────────────
      const dup = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_DUP,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(dup.code, 0, `第二轮 CLI 退出码（stderr: ${dup.stderr.slice(0, 600)}）`);
      assert(
        dup.stdout.includes('这个键已经记过。'),
        `第二轮应跑完且模型看到拒因（实际: ${JSON.stringify(dup.stdout)}）`,
      );

      // ── C 普通工具（启用侧）：两条不同正文 ⇒ 两格便签 ────────────────────
      const notesOn = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_NOTES,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(notesOn.code, 0, `便签轮 CLI 退出码（stderr: ${notesOn.stderr.slice(0, 600)}）`);

      // ── D 事实源读数：启用侧两组事实 ────────────────────────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉「幂等键只落一条」与「普通工具照常落两条」。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const recorded = events.filter((e) => e.kind === 'idempotency.recorded');
      assertEq(
        recorded.length,
        1,
        `同一键两次（一次落格、一次被拒）⇒ 只有一条：${JSON.stringify(recorded.map((e) => e.payload))}`,
      );
      assertEq(recorded[0]?.payload?.key, KEY, '键原样落格');
      assertEq(
        recorded[0]?.payload?.value,
        VALUE_FIRST,
        `首写为准（不是第二次的 ${VALUE_SECOND}）：${JSON.stringify(recorded[0]?.payload)}`,
      );

      const created = events.filter((e) => e.kind === 'notes.created');
      assertEq(
        created.length,
        NOTES.length,
        `启用侧普通工具两条不同正文应落两条：${JSON.stringify(created.map((e) => e.payload))}`,
      );
      assertEq(created.map((e) => e.payload?.text), NOTES, '两条便签正文按写入序');

      // ── E 卸任：工具面消失，普通工具与普通工具读数逐字一致 ───────────────
      //    「可卸载」的可执行形式是**装配期**不构造（`plugin_enabled: false`）。
      const manifest = join(hd.homedir, 'idempotency', 'PLUGIN.yml');
      assert(existsSync(manifest), `幂等插件应已被装配出来（缺 ${manifest}）`);
      const kept = readFileSync(manifest, 'utf8')
        .split('\n')
        .filter((l) => !/^plugin_enabled\s*:/.test(l))
        .join('\n');
      writeFileSync(manifest, `${kept.trimEnd()}\nplugin_enabled: false\n`);
      await llm.reset();

      const off = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_OFF_NOTES,
        provider: PROVIDER_ID,
        session: SID_OFF,
      });
      assertEq(off.code, 0, `停用后 CLI 应正常退出（stderr: ${off.stderr.slice(0, 600)}）`);

      const offTools = firstMainTools(await llm.requests());
      assert(offTools, '停用后仍应有主线请求（系统照常运行）');
      assert(
        !offTools.includes('idempotent_record'),
        `停用后工具面不该再有 idempotent_record（实际: ${JSON.stringify(offTools)}）`,
      );
      assert(offTools.includes('notes_add'), '停用后普通工具（notes_add）应在场');
      assert(off.stdout.includes('好了。'), `停用后这一轮仍应跑完（实际: ${JSON.stringify(off.stdout)}）`);

      // 停用侧同样直读 WAL：普通工具的领域事实照常两条（与启用侧逐字一致）。
      const offEvents = readFileSyncSafe(
        join(hd.homedir, 'session', SID_OFF, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));
      const offCreated = offEvents.filter((e) => e.kind === 'notes.created');
      assertEq(
        offCreated.length,
        NOTES.length,
        `停用侧普通工具两条不同正文也应落两条：${JSON.stringify(offCreated.map((e) => e.payload))}`,
      );
      assertEq(offCreated.map((e) => e.payload?.text), NOTES, '停用侧便签正文按写入序');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
