# 审计脚本与决策文档合理性复核（2026-09-20）

> **文档类型：审计** — 对「审计体系自身」与 ADR 的一次交叉复核。
> 复核对象：`scripts/` 下 8 个判定型守卫 + 1 个报告型守卫 + `gate.mjs` 编排，
> 以及 `docs/DECISIONS.md` 全部 19 条 ADR。
> 判据不是"有没有写"，而是**"写了的约束是否真的能红"**——一条不会红的约束
> 与没有约束等价，而且更危险：它让人以为那里有人看着。

## 复核方法

- 逐个读脚本头部的规则表与「已知边界」，再读实现，比对**宣称**与**判定**是否一致；
- 对 ADR 里每条**可验证的数字与"已删除 / 已实现"声明**去代码里实测，不采信文档自述；
- 对守卫的严重度分级（ERROR / WARNING）与 `gate.mjs` 的调用方式（是否带 `--strict`）
  交叉核对，得出"真正会让 CI 红"的规则清单。

---

## 一、结论速览

| # | 疑点 | 性质 | 严重度 |
|---|---|---|---|
| 1 | `commit-msg` 约束从未生效：钩子没装、CI 不跑 | **约束只存在于文档** | 高 |
| 2 | 8 个判定型守卫里 **3 个没有回归测试**，与 gate 自称的教条矛盾 | 教条执行不全 | 高 |
| 3 | CI 的 `paths` 触发器漏掉 `*.css` / `*.yml` / `*.html` | **整类改动零检查** | 高 |
| 4 | `test-layout-audit` 对"测试独立成文件"零判定，且统计口径把两类文件混算 | 名不副实的守卫 | 高 |
| 5 | `OPTION_PICK_FILE` 的豁免理由不成立：前端那处是**第二份真相**，不是"消费方" | 豁免掩盖了跨栈重复 | 中高 |
| 6 | `schema-audit` 报出的死项（`SchemaResponse`）无人处置 | 报告型守卫无闭环 | 中 |
| 7 | ADR-019 / ADR-010 / ADR-012 / ADR-017 四处事实漂移（已实测、已修正） | 文档漂移 | 中 |
| 8 | ADR-014「修订」段仍在描述已删除的 `tract` | 死知识伪装成现行约束 | 低 |

---

## 二、守卫自身的结构性缺口

### 2.1 `commit-msg` 钩子：从记忆搬进机制，但机制没插上电

`scripts/git-hooks/commit-msg` 的文件头写着「把『提交格式要求』**从记忆搬进机制**」，
`CONTRIBUTING.md:88` 也写了启用步骤 `git config core.hooksPath scripts/git-hooks`。
但实测：

- `git config --get core.hooksPath` → **空**；`.git/hooks/` 下全是 `*.sample`；
- `ci.yml` 的四个作业**没有一个**调用 `check-commit-msg.mjs`。

于是这条约束的实际执行方式是：**每个提交者记得手动跑一次脚本**。它又被搬回记忆里了，
而且比最初更隐蔽——因为文档里写着"已机制化"，没人会再去怀疑它。

仓库里所有提交确实都合规，那不是因为门禁在拦，是因为写提交的人（含 agent）每次都
自觉跑了 `check-commit-msg.mjs --file`。**一个靠自觉维持的门禁，等于一条写在文档里的
建议。**

### 2.2 「每个判定型守卫都先跑自己的回归测试」——8 个里只做了 5 个

`gate.mjs` 的 docs 阶段注释写得毫不含糊：

> 判定型守卫的**回归测试**必须先跑：一个只会亮绿灯的守卫等于没有守卫，
> 而它腐烂的方式恰恰是「规则写错了所以永远不命中」——只有注入真实违规
> 并断言脚本变红，才能把「通过」和「没在工作」区分开。

而它实际跑回归测试的只有 **5 个**（`grep-audit` / `mechanism-audit` /
`plugin-entry-audit` / `protocol-mirror-audit` / `dead-code-audit`，外加共享库
`color`）。**缺 3 个**：

| 守卫 | 有回归测试 | 备注 |
|---|---|---|
| `style-audit` | ✗ | 规则 B / C 本身也只是 WARNING |
| `doc-link-audit` | ✗ | 整体豁免 `docs/archive/`（39 文件） |
| `test-layout-audit` | ✗ | 见 §2.4，对核心约定零判定 |

这不是疏漏的随机分布：**缺回归测试的恰恰是判定力度最弱的那三个**。教条写得越响，
它没覆盖到的地方就越危险——因为读者会默认教条已经生效。

### 2.3 CI 的 `paths` 触发器漏掉三类文件

`.github/workflows/ci.yml` 的 `on.push.paths` / `on.pull_request.paths` 只列了
`*.rs` / `*.ts` / `*.vue` / `Cargo.*` / `package.json` / `rust-*.*` / `clippy.toml` /
`.editorconfig` / `scripts/**`。**没有** `*.css`、`*.yml`、`*.html`（`*.md` 同理）。

后果是具体的：

- `tauri/src/styles/` 下 **4 个 CSS 文件**（`base` / `controls` / `markdown` / `tokens`）
  若被单独修改，**四个 CI 作业一个都不跑** ⇒ `style-audit` 那条"使用的类必须有定义"
  的 ERROR 级规则，对纯 CSS 改动**完全不生效**；
- `PLUGIN.yml` 系列配置文件同理（`plugin-entry-audit` 的 E-006 是唯一相关规则，而它
  只是 WARNING）；
- `tauri/index.html` 同理。

`paths` 过滤的本意是省 CI 时间，但它隐含一个假设：**没被列进来的文件不会影响任何检查**。
CSS 这条反例直接证伪了它。

### 2.4 `test-layout-audit`：约定写得清清楚楚，却对它自己的违反零判定

脚本头部的约定是「测试独立成文件、同级同名加 `.test`」。它的两条检查是：

1. 已拆分的测试文件 → 宿主必须存在且声明 `#[path]`（**ERROR**）；
2. 内联 `mod tests` **在文件中部** → 提示拆分时别切错（**WARNING**）。

也就是说：**"根本没拆分"不构成任何违规**。只要内联块写在文件末尾，脚本报
`✓ 布局符合约定`。于是这条约定**没有棘轮**——新建一个带内联 `mod tests` 的生产文件
不会让任何东西变红，存量也不会被推着收敛。

附带一个统计口径错误：它报告的「含内联 mod tests 的文件：**106**」用的是正则
`/^\s*mod tests\b/m`，把两类完全不同的文件混在一起：

| 形态 | 数量 | 说明 |
|---|---|---|
| 宿主文件声明 `#[path = "X.test.rs"] mod tests;` | 53 | **合规**，正是约定要求的写法 |
| 真正的内联 `mod tests { … }` | 53 | 才是"未拆分" |

两者相加恰好 106。这个数字因此既不能说明"已拆分多少"，也不能说明"还剩多少未拆"。

### 2.5 一条**不成立的豁免理由**：把"第二份真相"说成"跨栈消费"

`symbio/src/symbio_core/schemas/options.rs:76`：

```rust
/// Rust 侧暂无动作声明它，**消费方在前端**：`useSessionOptions.ts::PICK_FILE`
/// 实现该原语……
#[allow(dead_code)] // dead-code-allow R-001: 闭集成员，消费方在前端 useSessionOptions.ts::PICK_FILE
pub const OPTION_PICK_FILE: &str = "file";
```

实测前端：`tauri/src/composables/useSessionOptions.ts:37` 是
`const PICK_FILE = 'file'`——一个**独立硬编码的字面量**，与 Rust 常量之间**没有任何
引用或校验关系**。所以"消费方在前端"这句话在字面意义上不成立：前端不是这个常量的
消费方，它是这个词的**第二份抄本**。

而且它援引的先例恰恰相反：`SESSION_CHAT_ABORT` 在前端是
`import { CHAT_ABORT } from '@/constants/pluginPaths'`，受 E-003 / E-008 等规则看管；
而 `OPTION_PICK_FILE` 不在任何守卫的登记里（A 组只认 `VDFS_*`，C 组只认
`rename_all` 枚举，**这个裸常量两边都够不着**）。

危害是：这条豁免把一个**无人看守的跨栈重复**正式登记成了"刻意保留的跨语言契约半边"，
于是下一个读代码的人不会再质疑它。**豁免写错了理由，比不写豁免更糟**——不写至少还有
人觉得可疑。

### 2.6 报告型守卫没有闭环

`schema-audit` 报出两条死项，其中
`symbio/src/symbio_core/schemas/common.rs:8 :: struct SchemaResponse`
实测**全仓零引用**（仅有 `mod.rs` 的一行 `pub use` 再导出），既没被删、也没在任何地方
登记豁免。它就这么一直挂在报告里——报告型守卫的产出**默认无人处理**，而它被接进门禁的
方式只有"崩溃即红"，所以这份报告挂多久都不会有人被提醒。

（另一条 `OPTION_PICK_FILE` 已按 §2.5 讨论。）

---

## 三、ADR 的事实漂移（均已实测，本次已就地修正）

| ADR | 文档原写 | 实测 | 处置 |
|---|---|---|---|
| ADR-019 | C 组守 `rename_all = "snake_case"` 枚举，4 张词表 | 现按声明取值自动分派（`snake_case` / `lowercase`），**6 张**词表 | 已改 |
| ADR-010 | 「只剩 `schemas/entities.rs` 里的 `DetailDefinition`」 | 该文件**已不存在**（随 ADR-011 删除），`DetailDefinition` 现住 `plugins/*/detail.rs` | 已改 |
| ADR-012 | 「列表类多列 **7** 个条目名」 | `JSON_DIGEST_MAX_NAMES = 8`（`context_window.rs:116`），且同文件注释也写 8 | 已改 |
| ADR-017 | session **14340 行 / 29.3%**，测试 **7044 行** | 2026-09-20 复测：**14517 行 / 25.3%**（总计 57426），测试 **7021 行** | 已改 |

ADR-017 的另一条断言「全部在独立 `*.test.rs` 里，内联为 0」**仍然成立**
（实测 `plugins/session` 下 `mod tests {` 命中 0），故保留未动。

**为什么这类漂移值得单独记一笔**：ADR-012 立下的正是"事实从代码提取、不从人手抄"的
原则，而 ADR 正文里那些具体数字恰恰是**手抄的**。原则管住了 `CURRENT.md`，没管住
ADR 自己。

### 附：ADR-014 的「修订」段是死知识

ADR-014 的 `### 修订（2026-09-18）` 整段（两条 tract 硬约束 + 一张数值对照表）描述的
是 `tract-onnx 0.23.7` 的行为，而 tract 已随 ADR-016 彻底退出依赖树。它作为决策记录
保留是合理的，但它**没有作废标记**，读起来与现行约束无法区分。ADR-014 顶部有推翻声明，
但只覆盖了"用 tract 做推理"与"性能"两点，没点名这一段。

---

## 四、判定力度偏弱——但**是有意为之**，不建议改

这一节列出的是"看见了但认为不该动"的项，避免把它们与上面的缺口混为一谈。

| 规则 | 现状 | 为什么保留 |
|---|---|---|
| `grep-audit` S-002-bonus（`let _ = ...await` 吞错） | 当前树 **27 处** WARN，退出码仍 0 | 它判的是"疑似"，误报会逼出豁免注释；一个靠豁免活着的守卫等于没有守卫 |
| `plugin-entry-audit` E-005 / E-006 | WARNING | 运行期拼路径数不出来，「定义了但没人用」只能报给人看 |
| `plugin-entry-audit` E-008 | 全仓只有 **2 处** `<!-- vocab: -->` 标记 | 覆盖面确实薄，但它是**可扩展**的：加标记即纳入，不是设计缺陷 |
| `style-audit` 规则 B / C | WARNING（需 `--strict`） | 见文件头「宁可漏报 unused，不误报 used」 |
| `dead-code-audit` R-001 | 只抓「整仓零引用」 | 弱引用（文档 / 注释 / 运行期拼名）一律视为存活，宁可漏报 |
| `schema-audit` | 报告型，仅防崩溃 | 报告内容需人工判断，机器判死会误伤 |
| `DYNAMIC_NAMESPACES`（6 个插件）E-005 不判定 | 白名单 | 把 `local` 判成死路由会让审计立刻失去可信度 |

**共同点**：它们都在"误报 → 逼出豁免 → 守卫失效"这条链上做了自觉的让步。
这条让步本身是对的。真正的风险不在这些规则弱，而在 **§2 那几条：弱之外还被误以为强**。

---

## 五、核对后**成立**的（避免过度质疑）

复核不能只挑毛病。以下声明一度被列为嫌疑，实测后确认无误：

- **ADR-008**：`store_kind` / `create_store` 确已删除（仅存于 `config.rs` 与
  `store/mod.rs` 的历史说明注释，且注释本身写得很清楚）；
- **ADR-009**：`CapabilityCategory::Metacognition` 确仍存在（`capability.rs:24`）；
- **ADR-006**：Tauri 命令确为 3 个（`route_v2` / `route_v2_send` / `route_v2_close`，
  `meta` 已注释停用）；
- **ADR-015**：前端生产代码里确无 `switch (event.type)`（两处命中都在注释里，
  且注释正是在解释"为什么没有"），`sessionBusWatcher.ts` 确已删除；
- **ADR-013 / ADR-016**：`native-tls` 与 `ort = "2.0.0-rc.13"` 均在 `Cargo.toml` 中，
  `tract` 确已退出；
- **ADR-004**：4 套协议适配器目录确在 `plugins/model/protocols/`。

---

## 六、建议的处置顺序

按"先堵最像约束的假约束"排序，每一步都可独立提交：

1. **装上钩子**（一行：`git config core.hooksPath scripts/git-hooks`），或把它接进
   CI（推荐后者——本机配置管不住别人）。接进 CI 需要新增一个作业或在
   `docs-validation` 里加一步对最近提交信息的校验。
2. **补齐 3 个守卫的回归测试**（`style-audit` / `doc-link-audit` /
   `test-layout-audit`），让 gate 的教条名实相符；补的过程会顺带暴露它们各自的
   判定盲区。
3. **给 CI 的 `paths` 补 `*.css` / `*.yml` / `*.html`**，或改用
   `paths-ignore` 只排除明确的产出目录（`target/` / `dist/` / `vendor/`）——后者更稳，
   不会每次新增文件类型都再漏一次。
4. **修 `OPTION_PICK_FILE` 那条豁免**：要么把 `'file'` 纳入 C 组（需要后端把它变成
   枚举或加入常量镜像清单），要么把豁免理由改成如实描述（"前端另有一份字面量，
   暂无守卫"）。后者只是诚实，前者才解决问题——但它**涉及后端**，按约定须先整体规划。
5. **给 `SchemaResponse` 一个处置**（删除或登记豁免理由），并考虑让报告型守卫的
   "长期未处置项"有个可见出口。
6. 给 ADR-014 的「修订」段加作废标记。

---

> **复核边界**：本文的实测数字取自 2026-09-20 的工作树，度量口径已在各处注明
> （生产代码 = `*.rs` 排除 `*.test.rs` 与 `tests.rs`）。数字会漂移，结论不会。
> 若将来要复核本文，重跑 `node scripts/gate.mjs --only=docs,facts` 与
> `node scripts/schema-audit.mjs` 即可核对绝大多数条目。
