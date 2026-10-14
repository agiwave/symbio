import './_selfrun.mjs';
// T56 信封契约（LC-001）：provider 配置怎么配，发给模型的请求就怎么带。
//
// ## 本用例钉的是什么
//
// 「配了不用」是整条链路上最安静的一类失效：模型照常回答、CLI 照常退出 0，
// 只有请求体里的 `model` / `temperature` / `max_tokens` 还是旧值或丢了。
// 响应侧没有任何东西会红——所以判据必须从**请求体**取（mock-llm `/_requests`）。
//
// 三条各自独立可红：
// ① 出厂配置逐字段进请求（`providerConfig` 的四个值 + stream 恒 true）；
// ② 改配置 ⇒ 下一个进程的请求跟着改（**反向自检**：把断言钉死在常量上，
//    这步就会红——它证明判据读的是配置不是字面量）；
// ③ `tool_choice` 与工具面同步（有工具 ⇒ auto；辅助请求无工具 ⇒ 键不出现）。
//
// ## 为什么换配置要起新进程
//
// provider 目录在 traverse 注册期解析（`model/plugin.rs` 按 PROVIDER_ID 定生效者），
// 进程内改文件不重解析。起新进程 = 用户重启应用的真实形态，也是判据唯一
// 能触及「配置 → 请求」这条因果的时机。
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';
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
} from '../helpers.mjs';
import { assertEnvelope, assertStreamOnly, assertToolChoiceSync } from '../llm-contract.mjs';

export default defineCase(
  'T56 信封契约：provider 配置逐字段进请求 + 改配置可见生效 + tool_choice 同步（LC-001）',
  async () => {
    const llm = await new MockLlm([
      { id: 'a', match: '第一轮', content: '回答甲。' },
      { id: 'b', match: '第二轮', content: '回答乙。' },
    ]).start();

    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 主题是信封，与对话面正交 ⇒ 三个开关钉死（理由见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF },
      },
    });

    const providerFile = join(hd.homedir, 'model', PROVIDER_ID, 'provider.json');

    try {
      // ── ① 出厂配置逐字段进请求 ─────────────────────────────────────────
      const r1 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '第一轮：打个招呼',
        provider: PROVIDER_ID,
        session: 'e2e-t56',
      });
      assertEq(r1.code, 0, `CLI 退出码（stderr: ${r1.stderr.slice(0, 600)}）`);

      const reqs1 = await llm.requests();
      assertEq(reqs1.length, 1, '第一轮应恰好一次请求（对话面已关）');
      // providerConfig 的出厂值：model / temperature 0.1 / max_tokens 1024。
      assertEnvelope(
        reqs1[0],
        { model: 'mock-model', temperature: 0.1, max_tokens: 1024 },
        'T56① 出厂信封',
      );
      assertStreamOnly(reqs1[0], 'T56①');
      // 主线轮次带出厂工具面 ⇒ tool_choice 必须是 auto（缺它 = 模型不调工具，
      // 而响应侧只表现为「这轮没干活」）。
      assertToolChoiceSync(reqs1[0], 'T56①');

      // ── ② 改配置 ⇒ 新进程的请求跟着改（反向自检的那一步）────────────────
      // 四个值全换：任何一处「钉在常量上」的退化都会在这里红。
      const changed = providerConfig(llm.port, {
        model: 'mock-model-v2',
        temperature: 0.7,
        max_tokens: 2048,
      });
      writeFileSync(providerFile, JSON.stringify(changed));

      const r2 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '第二轮：再打个招呼',
        provider: PROVIDER_ID,
        session: 'e2e-t56',
      });
      assertEq(r2.code, 0, `CLI 退出码（stderr: ${r2.stderr.slice(0, 600)}）`);

      const reqs2 = await llm.requests();
      assertEq(reqs2.length, 2, '第二轮应累计两次请求');
      assertEnvelope(
        reqs2[1],
        { model: 'mock-model-v2', temperature: 0.7, max_tokens: 2048 },
        'T56② 改配置后的信封',
      );
      assertStreamOnly(reqs2[1], 'T56②');
      assertToolChoiceSync(reqs2[1], 'T56②');

      // 辅助请求（无工具面 ⇒ 无 tool_choice）那半条同步规则归 t61——那里对话面
      // 显式全开，分类 / 措辞请求是现成的无工具面样本；本用例钉死对话面
      // （`DIALOG_FACE_OFF`）换来的请求数确定性，两边不互相借判据。
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);
