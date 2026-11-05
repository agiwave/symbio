import './_selfrun.mjs';
// T92 人格是一条**插件能力**：填一段人格 ⇒ 两条面向用户的输出面（轮内正文 / 对话
// 措辞）读到的系统提示词里都有它；摘掉插件 ⇒ 两处都没有，而系统照常运行。
//
// ## 本用例钉的是「新能力长在插件里，架构不动」
//
// 名册 #47「人格语气一致」（`docs/003-design/026-能力拆分与影响类.md` §2.3）判为
// **A 可插件**：它只需要今天已有的扩展面。本用例把这条判定变成可执行的三件事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 能力出现 | 首次模型请求的 system 里出现人格段（含 `【人格】` 标题与正文） |
// | **一致性** | 措辞那条**静默**请求的 system 里也有同一段，且排在指令段之前 |
// | 卸任后消失 | 停用插件后主线 system 不再含它，而这一轮照常落库（系统完整运行） |
//
// 「一致性」是本条的要害：两条输出面各自拼自己的系统提示词
// （`loop` 的 `resolve_system_prompt` 与 `compose` 的 `build_system_prompt`），
// 它们能读到同一段，唯一的原因是**人格只有一个 owner**（`family::PROMPT` 的一行）。
// 若某天有人把人格复制成两份常量，主线侧断言仍然绿，措辞侧那条会红。
//
// ## 为什么由请求体判，而不读后端状态
//
// 「人格送达了模型」只有模型那一侧看得见。系统里没有任何一处记录「本轮用的提示词
// 是什么」——那正是 prompt 装配的纪律（不存第二份）。因此判据取 mock 侧记录的
// 请求体：那是唯一的现象，也是真正重要的那一份（模型看到什么，就是什么）。
//
// ## 为什么停用走插件目录而不是源码
//
// `plugin_enabled` 是 `.symbio/<插件>/PLUGIN.yml` 上的运行期装配位（与 `plugin_manager`
// 的停用同一条路）。用例只翻这一个键，装配侧没有任何分支——「可卸载」因此不是
// 一句承诺，而是装配期「根本不构造」。
import { join } from 'node:path';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import { systemTextOf, envelopeOf, isSilentAuxRequest } from '../llm-contract.mjs';

/** 人格正文：一个不可能偶然出现在别处的标记，判据因此不必依赖上下文。 */
const PERSONA = 'T92：你说话简短，不寒暄，先给结论。';
/** 人格段的标题（`plugin.rs::segment_text` 的排版）——用它判「这一段是人格，不是 provider 的 system_prompt」 */
const SEGMENT_TITLE = '【人格】';

const U_WORK = '帮我把 README 读一下';
const U_DIRECT = '我们刚才聊了什么';

/** 措辞请求的识别标记（compose `wording.rs::INSTRUCTION` 首句） */
const COMPOSE_MARK = '你是这个助手本人';

/** 一轮「干活」+ 一轮「说到刚才」（后者会走措辞那条静默往返，见 t61 的手法） */
const TURNS_ON = `${U_WORK}\n${U_DIRECT}\n`;
const SID_ON = 'e2e-t92-on';
const SID_OFF = 'e2e-t92-off';

/** 把 `plugin_enabled: false` 写进插件清单（先删旧值，避免同名键出现两次） */
function disablePlugin(homedir, name) {
  const path = join(homedir, name, 'PLUGIN.yml');
  assert(existsSync(path), `插件 ${name} 应已被装配出来（缺 ${path}）`);
  const kept = readFileSync(path, 'utf8')
    .split('\n')
    .filter((line) => !/^plugin_enabled\s*:/.test(line))
    .join('\n');
  writeFileSync(path, `${kept.trimEnd()}\nplugin_enabled: false\n`);
}

/** 主线请求 = 带工具面的那条；辅助请求（分类 / 措辞）恒不带工具面 */
const mainRequests = (reqs) => reqs.filter((q) => !isSilentAuxRequest(q) && envelopeOf(q).hasTools);
const composeRequests = (reqs) =>
  reqs.filter((q) => !envelopeOf(q).hasTools && systemTextOf(q).includes(COMPOSE_MARK));

export default defineCase(
  'T92 人格：段同时送达轮内正文与对话措辞，停用后两处都消失而系统照常运行',
  async () => {
    const llm = await new MockLlm([
      // 分类先发生，用 `once` 消耗掉（与 t21 / t61 同一手法：分类请求与主线请求的
      // 末条 user 是同一句话，按序区分）
      { id: 'cls-work', match: U_WORK, content: 'work', once: true },
      { id: 'cls-direct', match: U_DIRECT, content: 'direct', once: true },
      { id: 'main', content: '主线回答：README 已读完。' },
      { id: 'compose', content: '我们刚才在聊 README 的事。' },
    ]).start();

    const hd = makeHomedir({
      // provider **不写** `system_prompt`：本用例要证明人格不依赖模型接入配置
      // （出厂形态下 provider 就没有人格段，人格此刻只可能来自本插件）。
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 对话面显式全开：本用例要读的就是「措辞」那条请求，不能依赖默认值。
        session: { classify_enabled: true, compose_enabled: true, progress_enabled: false },
        // 人格的**全部**输入：一段正文。没有它，本插件一条段都不交。
        persona: { persona: PERSONA },
      },
    });

    try {
      // ── A 能力出现：装配 + 配置落位 ──────────────────────────────────────
      const manifest = join(hd.homedir, 'persona', 'PLUGIN.yml');
      assert(existsSync(manifest), `人格插件应被装配出来（缺 ${manifest}）`);
      assert(
        readFileSync(manifest, 'utf8').includes(PERSONA),
        '人格正文应落在插件自己的配置文档里（`<根>/persona/PLUGIN.yml`）',
      );

      // ── B 两条输出面的读数 ───────────────────────────────────────────────
      const on = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID_ON,
        stdinText: TURNS_ON,
      });
      assertEq(on.code, 0, `CLI 退出码（stderr: ${on.stderr.slice(0, 600)}）`);

      const reqs = await llm.requests();
      const main = mainRequests(reqs);
      const compose = composeRequests(reqs);
      assertEq(main.length, 1, `应有 1 条主线请求（实际 ${main.length}）`);
      assertEq(compose.length, 1, `应有 1 条措辞请求（实际 ${compose.length}）`);

      // ① 轮内正文：人格段到场，且是**本插件排的那个版**（标题 + 正文）
      const mainSys = systemTextOf(main[0]);
      assert(mainSys.includes(SEGMENT_TITLE), `主线 system 应有【人格】段（实际前 300 字：${mainSys.slice(0, 300)}）`);
      assert(mainSys.includes(PERSONA), '主线 system 应带人格正文（段注册了但没送达 = 最安静的失效）');

      // ② 一致性：同一条静默路径（换一个插件、换一份提示词）也读到了同一段
      const composeSys = systemTextOf(compose[0]);
      assert(
        composeSys.includes(PERSONA),
        `措辞 system 应带同一段人格——否则同一个人在两处口吻不同（实际前 300 字：${composeSys.slice(0, 300)}）`,
      );
      const personaAt = composeSys.indexOf(SEGMENT_TITLE);
      const instrAt = composeSys.indexOf(COMPOSE_MARK);
      assert(
        personaAt >= 0 && instrAt >= 0 && personaAt < instrAt,
        '人格段必须排在措辞指令段之前（指令在后才压得住措辞，反过来就是「怎么写」压过「我是谁」）',
      );

      // ── C 卸任：段消失，而系统照常运行 ───────────────────────────────────
      disablePlugin(hd.homedir, 'persona');
      await llm.reset();

      const off = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        provider: PROVIDER_ID,
        session: SID_OFF,
        stdinText: `${U_WORK}\n`,
      });
      assertEq(off.code, 0, `停用后 CLI 仍应正常退出（stderr: ${off.stderr.slice(0, 600)}）`);

      const offReqs = await llm.requests();
      const offMain = mainRequests(offReqs);
      assertEq(offMain.length, 1, `停用后仍应有 1 条主线请求（实际 ${offMain.length}）`);
      const offSys = systemTextOf(offMain[0]);
      assert(!offSys.includes(SEGMENT_TITLE), `停用后不该再有【人格】段（实际前 300 字：${offSys.slice(0, 300)}）`);
      assert(!offSys.includes(PERSONA), '停用后人格正文必须消失——留在提示词里就是「卸载了还在生效」');
      // 「照常运行」的可执行形式：系统提示词仍在（不是被删空），且这一轮真的跑完了
      assert(offSys.trim().length > 0, '停用一个人格不该让系统提示词变空');
      assertEq(offReqs.filter((q) => !isSilentAuxRequest(q) && envelopeOf(q).hasTools).length, 1, '停用后主线轮次照常发生');
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
