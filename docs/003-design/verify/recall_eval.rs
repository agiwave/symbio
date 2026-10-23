//! 召回评测集 · recall@k（plan/13 批 P5）
//!
//! 本程序回答一个问题：
//!   **「存进了记忆」等于「用的时候找得回」吗？**
//!
//! 这是 G6「存了但找不到」的**可复现数字**。在它之前，召回通路只有口径判据
//! （排除式遗忘 / 放宽即召回 / 活参数三条），没有一个数字能回答
//! 「给定一个查询，前 k 条里有多少是用户当时想找的那条」。
//!
//! ## 本程序跑的是**生产排序内核**，不是它的临摹件
//!
//! 排序经 `#[path]` **包含生产文件**：[`recall_score`]（`symbio/src/symbio_core/
//! projection/recall_score.rs`——`recall` 投影的排序实现）。因此这里出的
//! recall@k 是**生产投影的口径**（ 批②③）：查询驱动的内容重叠分降序，
//! 平手 ts 降序、再平手 seq 降序。
//!
//! ## 代价：这套数字与生产绑死
//!
//! 评测数字的**唯一**价值来自它与生产有因果关系。把排序实现抄一份进本文件
//! （或写一个「理想召回器」当被测对象）能得到漂亮得多的分数，但生产换一种
//! 排序时那些分数一个都不动——临摹件随生产漂移而**静默失效**，正是 
//! 黑盒纪律点名要防的形态。代价是本文件不能脱离生产文件单独编译，换来的是
//! 「换排序 = 两边同时换」。
//!
//! 边界（诚实划界）：池成员资格（`tag` 过滤 / as-of / 排除式遗忘 / 预算）由
//! 投影的外壳负责，**不在本文件**——那些各有口径判据（`recall.test.rs` /
//! S06 §6 验收）。本程序钉的是**排序**：给定同一个池，生产把用户想找的那条
//! 排到多前。
//!
//! ## 评测集为什么是**数据**而不是断言
//!
//! 一个 recall@k 数字要可信，必须满足两条：
//!   1. 每个用例**独立可读**（查询是什么、期望命中哪一格、为什么是它）；
//!   2. **反向可改**——把某条的 query 改掉，那个用例的数字必须跟着掉
//!      （否则整套集合可能是恒真的）。
//!
//! 所以用例是显式表（见 `cases()`），主体只有三段：建记忆 → 召回 → 算 hit@k。
//!
//! ## 两个口径：n（样本量）与 pass^k（稳定性）
//!
//! 一个比率没有样本量就不是数字。规则三：n 次试验 0 失败 ⇒ 95% 置信下失败率
//! 上界 = 3/n。n=3 全过只能说出「上界 100%」，等于什么都没排除；n≥32 才让
//! 「成功率 ≥ 1−3/n > 0.90」成为一句可引用的话。所以 `main` 把 **n ≥ 32**
//! 断言成判据，用例数是样本量，不是体量装饰。
//!
//! 稳定性用 `pass^k`：一条用例在**同一问题的 k 种合法外观**下都命中才记 1 分。
//! 变体不是「同一份输入跑四遍」——排序内核是确定性函数，重复跑恒等，那种
//! pass^k 一个信息也不增加。真实变动的是输入外观：池的物理顺序、池里更多的
//! 更旧无关条目、同一条记忆被重复入池（见 `variants`）。
//!
//! ## 第四个数字：换述召回率与「依赖度」
//!
//! 前三个数说「同一批池外观下命中稳不稳」。第四个数问另一件事：
//! **同一句话换个说法，还找得回吗？**
//!
//! 做法是把查询换成 [`PARAPHRASES_TSV`] 里的同义换述（33 条用例各 ≥2 种说法，
//! 冻结在 `fixtures/`），池完全不动，再跑一遍 recall@k。两个数的差
//! `dependency = 原述 − 换述` 就是**依赖度**：越大，这套排序越靠字面。
//! 它是 [DE-008 §3.1.1](../008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") 那张表里
//! 「抽象层级」一格的离线效标——此前那一格写的是「本轴目前只能人工判」。
//!
//! ## 换述为什么不并进 `pass^k`
//!
//! 它是同一件事的**第三类输入外观**（池形状 V1–V3 之外的查询措辞），但方向
//! 相反：`pass^k` 要求「全部外观都命中」，而换述的**未命中本身就是读数**。
//! 把换述塞进 `variants()` 只有两种结局——要么把换述写得都会命中（读数恒为 1，
//! 白测），要么 `pass_all >= 0.90` 那条断言逼人删换述。所以它单独出一个数，
//! 不冒充稳定性。
//!
//! ## 这条读数**不**覆盖什么（写在这里，也写进 DE-008 那一格）
//!
//! 1. 它测的是**排序器对措辞的敏感度**，不是「系统会归纳」。读数低只说明
//!    「这套排序不靠字面」，不说明任何东西理解了同义。
//! 2. 换述是人写的 ⇒ 它只是效标的**下界**：换述越接近原述差值越小越好看。
//!    因此换述表带指纹锁（改一个字节就红），重叠均值也一并打印。
//! 3. 有一部分用例**结构上**与措辞无关（时间兜底那类：期望的就是最新那条，
//!    任何查询都命中）。它们只能进分母，不能进依赖度的叙述——程序把这一点
//!    算成两个数并逐条验证（见 `main` 的第 9、10 条判据）。
//!
//! 论证纪律：每条断言配反向用例（改输入，结论必须变）。
//!
//! 编译运行：rustc --edition 2021 recall_eval.rs -o re && ./re

// ── 生产排序内核（唯一共有实现， 批③）─────────────────────────────────
//
// `#[path]` 指向仓库里的生产文件——**这一行就是「跑的是生产」的全部含义**。
// 它被 `recall` 投影与召回评测集共同编译：改排序只有一处可改，两边同时变。
#[path = "../../../symbio/src/symbio_core/projection/recall_score.rs"]
mod recall_score;

use recall_score::{rank, ScoreEntry};

// ───────────────────────── 领域类型 ─────────────────────────

/// 一条记忆（本程序的最小形状：内容 + 标签 + 时间戳）。
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    /// 记忆正文（将来的 embedding 落在 content 上，01 §9.2）
    pub content: String,
    /// 标签（「记忆类型」：召回的唯一**读侧过滤**参数）
    pub tag: &'static str,
    /// 写入时刻（越新越靠前）
    pub ts: u64,
}

/// 一个评测用例。
#[derive(Debug, Clone)]
pub struct Case {
    /// 用例名（也是失败时的定位符）
    pub name: &'static str,
    /// 查询（用户「想找什么」的自然语言）
    pub query: &'static str,
    /// 记忆池（同一池内建多条，只有一条是用户想找的）
    pub memories: Vec<Memory>,
    /// 期望命中的格子：`memories` 里那条的**下标**
    pub expect: usize,
    /// k（前几条算命中）
    pub k: usize,
}

// ───────────────────────── 召回主体 ─────────────────────────

/// 把评测池喂给**生产排序内核**：`query` 驱动内容匹配（生产读侧
/// `memory_events::recall_view` 的口径），seq 用池下标（全序 ⇒ 确定性）。
fn production_recall(pool: &[Memory], query: &str, k: usize) -> Vec<usize> {
    let entries: Vec<ScoreEntry> = pool
        .iter()
        .enumerate()
        .map(|(i, m)| ScoreEntry {
            content: m.content.clone(),
            tag: m.tag.to_string(),
            ts: m.ts as i64,
            seq: i as u64,
        })
        .collect();
    rank(entries, Some(query))
        .into_iter()
        .take(k)
        .map(|e| e.seq as usize)
        .collect()
}

/// 平凡值路径：同一内核在无查询时 = 纯 ts 降序（ 批②的 J2 判据）。
fn trivial_recall(pool: &[Memory], _query: &str, k: usize) -> Vec<usize> {
    let entries: Vec<ScoreEntry> = pool
        .iter()
        .enumerate()
        .map(|(i, m)| ScoreEntry {
            content: m.content.clone(),
            tag: m.tag.to_string(),
            ts: m.ts as i64,
            seq: i as u64,
        })
        .collect();
    rank(entries, None)
        .into_iter()
        .take(k)
        .map(|e| e.seq as usize)
        .collect()
}

// ───────────────────────── 评测集 ─────────────────────────

/// 一条「相关但更旧」的标准用例：池里第 0 条是用户想找的那条（**最旧**），
/// 其余是更近但无关的记忆。纯时间序必然把它压到 k 之外，内容匹配把它顶上来。
///
/// 池序即 seq 序（`production_recall` 用下标作 seq），所以「最旧」与「最靠前」
/// 是两件事：目标在池首、ts 最低——两条口径在这里是**相反**的。
fn stale_relevant(
    name: &'static str,
    query: &'static str,
    target: &str,
    distractors: &[&str],
) -> Case {
    let mut memories = vec![Memory { content: target.to_string(), tag: "经验", ts: 1_000 }];
    for (i, d) in distractors.iter().enumerate() {
        memories.push(Memory {
            content: d.to_string(),
            tag: "经验",
            ts: 9_000 + i as u64 * 100,
        });
    }
    Case { name, query, memories, expect: 0, k: 1 }
}

/// 一条「完全无关」的用例：查询与池里任何一条都没有词重叠，任何实现都只能
/// 给最近的那些——这一条钉的是**不许空手而归**（I3），期望下标 = 最新那条。
fn recency_fallback(
    name: &'static str,
    query: &'static str,
    pool: &[&str],
) -> Case {
    let memories: Vec<Memory> = pool
        .iter()
        .enumerate()
        .map(|(i, c)| Memory {
            content: c.to_string(),
            tag: "经验",
            ts: 2_000 + i as u64 * 1_000,
        })
        .collect();
    Case { name, query, memories, expect: pool.len() - 1, k: 1 }
}

/// 用例集。**每条都要能独立读懂**——查询是什么、池里有哪几条、期望是哪条。
///
/// 设计取向：让「内容匹配」与「纯时间序」**恰好在这批用例上分胜负**。
/// 目标条一律放在池首且 ts 最低：内容口径要把它顶到第 1，时间口径会把它压到最后。
///
/// ## 为什么要有 n（而不是三条）
///
/// `recall@1` 是一个**比率估计**，三条全过在 95% 置信下的失败率上界仍是 100%
/// （规则三：0 次失败 ⇒ 上界 3/n）。本集合的规模因此是判据的一部分：
/// `main` 断言 `n ≥ 32`，于是「全过」可以说出 `成功率 ≥ 1 − 3/n > 0.90` 这句话。
/// 加用例不是把表撑大——每条都必须**独立可分辨**，不分辨的用例比没有更坏。
pub fn cases() -> Vec<Case> {
    let mut v = Vec::new();

    // ① 经典「存了但找不到」：要找的那条**最旧**，另有更新的同 tag 记忆。
    //    纯时间序必然把它排到后面；内容匹配把它顶到第一。
    v.push(stale_relevant(
        "stale-but-relevant loses to fresh-but-irrelevant under time order",
        "部署时数据库连接池打满的处理",
        "部署时数据库连接池打满：把最大连接数从 20 调到 50，并给慢查询加索引",
        &["今天改了 README 的一段说明", "调整了日志级别，减少噪声"],
    ));

    // ② 词部分重叠：期望那条共享两个查询词，另一条只共享一个。
    v.push(Case {
        name: "partial word overlap ranks the richer match first",
        query: "内存泄漏怎么定位",
        memories: vec![
            Memory { content: "用 heaptrack 定位内存泄漏，看谁的引用没释放".into(), tag: "经验", ts: 5_000 },
            Memory { content: "内存占用偏高时不一定是泄漏，先看缓存大小".into(), tag: "经验", ts: 9_000 },
        ],
        expect: 0,
        k: 1,
    });

    // ③ 完全无关的查询：任何实现都只能给"最近的那几条"——这一条钉的是
    //    **不许空手而归**（I3：静默超时 / 空且不标降级都算失败形态），
    //    同时 k=1 让「时间序兜底」成为**可分辨**的判据。
    //    ⚠️ k=2 是错的第一版：池里只有两条 ⇒ 任何排序都全命中，这条用例
    //    于是对"看内容 / 不看内容"零分辨力，整套集合的 content 与 time 分数
    //    就变得一模一样（评测跑出来正是 0.333 = 0.333）。**一条不分辨的用例
    //    比没有更坏**——它让集合看起来更大，却不增加任何判别力。
    v.push(recency_fallback(
        "an unrelated query falls back to recency, never to silence",
        "量子计算的纠错码",
        &["前端构建慢：把 sourcemap 关掉", "接口超时：把重试从 3 次改成 5 次"],
    ));

    // ④–㉙ 规模化：每条都是「相关但更旧」或「无关兜底」，覆盖运维 / 数据 /
    //    前端 / 工程流程四类查询，词表不重复（一条用例的分词结果不解释另一条）。
    let mut pool: Vec<Case> = Vec::new();

    pool.push(stale_relevant(
        "crashloopbackoff-diagnosis",
        "Kubernetes pod 一直 CrashLoopBackOff 怎么排查",
        "Kubernetes pod 一直 CrashLoopBackOff：先 kubectl logs --previous 看上次退出的栈，再看 readiness 探针是否误配",
        &["把 helm chart 的版本号从 1.2 升到 1.3", "CI 的超时从 10 分钟改成 20 分钟"],
    ));
    pool.push(stale_relevant(
        "cache-breakdown-lock",
        "Redis 缓存击穿让数据库压力骤增",
        "Redis 缓存击穿：热点 key 加互斥锁 + 逻辑过期，避免同一时刻全部回源到数据库",
        &["换了台显示器，颜色偏了", "周报模板改了标题格式"],
    ));
    pool.push(stale_relevant(
        "nginx-502-locate",
        "Nginx 502 网关错误的定位顺序",
        "Nginx 502 定位顺序：先看 upstream 是否存活，再看 proxy_read_timeout 与 worker 连接数",
        &["把会议室投影的分辨率调低", "整理了下载目录里的安装包"],
    ));
    pool.push(stale_relevant(
        "python-memory-growth",
        "Python 脚本内存占用一直涨",
        "Python 脚本内存占用一直涨：用 tracemalloc 取快照对比，多数是全局缓存与闭包持有",
        &["换了键盘的键位映射", "把打印机的双面打印打开"],
    ));
    pool.push(stale_relevant(
        "git-rebase-conflict",
        "Git rebase 冲突反复出现的处理",
        "Git rebase 冲突反复出现：先 rerere 记录解法，再把格式改动与语义改动拆成两个提交",
        &["给文档加了一条脚注编号", "把桌面的壁纸换成深色"],
    ));
    pool.push(stale_relevant(
        "sql-slow-scan",
        "SQL 慢查询没有走索引",
        "SQL 慢查询没有走索引：用 EXPLAIN 看 type，隐式类型转换与前置通配符都会让索引失效",
        &["重排了书架的层高", "把手机的自动锁屏时间调长"],
    ));
    pool.push(stale_relevant(
        "first-paint-three-seconds",
        "前端首屏加载三秒以上",
        "前端首屏加载三秒以上：先拆分首屏包的体积，再看图片是否缺尺寸与懒加载，最后才考虑 SSR",
        &["把文档里的表格列宽调匀", "换了一个更轻的代码字体"],
    ));
    pool.push(stale_relevant(
        "docker-cache-miss",
        "Docker 构建缓存每次失效",
        "Docker 构建缓存每次失效：COPY 的顺序把易变文件放后面，依赖安装与源码复制分成两层",
        &["把工位的椅子升高两厘米", "整理了收件箱里的订阅邮件"],
    ));
    pool.push(stale_relevant(
        "tls-cert-expiry",
        "TLS 证书过期告警没人处理",
        "TLS 证书过期告警：加一条到期前 30 天的自动续签任务，并把续签结果写进部署日志",
        &["把日历的第一天改成周一", "调整了输入法的候选词数量"],
    ));
    pool.push(stale_relevant(
        "mq-backlog",
        "消息队列积压消费不过来",
        "消息队列积压消费不过来：先看消费端是否卡在单条慢任务，再考虑扩分区而不是无限加消费者",
        &["把桌面的图标按类型归类", "重命名了几个下载的压缩包"],
    ));
    pool.push(stale_relevant(
        "flaky-unit-test",
        "单元测试偶发失败怎么查",
        "单元测试偶发失败：先看是否有共享可变状态与时间依赖，再固定随机种子与时钟",
        &["把网页的缩放比例调回 100%", "给笔记本换了一块更大的硬盘"],
    ));
    pool.push(stale_relevant(
        "duplicate-alerts",
        "日志里出现大量重复告警",
        "日志里出现大量重复告警：按指纹做去重窗口，同时把根因那条单独标出来",
        &["把邮件签名里的职位改了", "安装了新的截图工具"],
    ));
    pool.push(stale_relevant(
        "p99-jitter",
        "接口 P99 抖动严重而 P50 正常",
        "接口 P99 抖动严重而 P50 正常：多为垃圾回收停顿与连接池排队，先看分位数的分布尾部",
        &["把耳机线材换成了编织线", "整理了桌面的文件收纳盒"],
    ));
    pool.push(stale_relevant(
        "backup-restore-drill",
        "备份恢复演练没有通过",
        "备份恢复演练没有通过：先验备份文件的完整性校验，再验恢复后的数据一致性，缺一步都算没通过",
        &["把书签按文件夹重新分组", "给路由器换了个位置"],
    ));
    pool.push(stale_relevant(
        "timezone-cron",
        "时区导致定时任务跑错时间",
        "时区导致定时任务跑错时间：调度统一用 UTC 存、按会话时区展示，别在 cron 里写本地时间",
        &["把壁纸换成纯色背景", "给风扇清了一下灰尘"],
    ));
    pool.push(stale_relevant(
        "batch-import-timeout",
        "批量导入接口超时",
        "批量导入接口超时：改成分片提交 + 幂等键，把超时判据从单请求挪到分片",
        &["把文档的页边距调宽", "更换了一个更安静的鼠标"],
    ));
    pool.push(stale_relevant(
        "mobile-upload-fail",
        "移动端图片上传失败",
        "移动端图片上传失败：先看是否被网关的请求体上限截断，再压缩到长边 1600 以内",
        &["把相册按拍摄地点分了组", "给手机换了个透明外壳"],
    ));
    pool.push(stale_relevant(
        "config-hot-reload",
        "配置中心热更新不生效",
        "配置中心热更新不生效：订阅要落在长连接上，且读侧缓存必须有失效回调，否则只更新了副本",
        &["把窗帘的挂钩换了位置", "整理了厨房的调料罐"],
    ));
    pool.push(stale_relevant(
        "websocket-reconnect",
        "WebSocket 连接频繁断开",
        "WebSocket 连接频繁断开：先看中间代理的空闲超时，再看心跳间隔是否大于它",
        &["把沙发的靠垫换了套子", "给水壶除了一次垢"],
    ));
    pool.push(stale_relevant(
        "inode-exhausted",
        "磁盘 inode 耗尽但空间还有",
        "磁盘 inode 耗尽但空间还有：多是海量小文件，用 du + 文件数排序找出目录，再收敛日志轮转",
        &["把抽屉里的电池按型号摆好", "给台灯换了个暖光灯泡"],
    ));
    pool.push(stale_relevant(
        "review-same-rework",
        "代码评审总在同一个点上返工",
        "代码评审总在同一个点上返工：把那条约定写成门禁判据，评审就不再靠人记得",
        &["把会议纪要的模板换了", "给工位加了一块垫板"],
    ));
    pool.push(stale_relevant(
        "requirement-churn",
        "需求变更导致反复返工",
        "需求变更导致反复返工：把变更点做成可回退的切片，先落最小可验证版本再扩",
        &["把台历翻到了下个月", "换了一支更细的中性笔"],
    ));
    pool.push(stale_relevant(
        "env-mismatch",
        "本地开发环境与线上不一致",
        "本地开发环境与线上不一致：把运行时版本与依赖锁写进仓库，并用同一份镜像跑测试",
        &["把文件夹的排序改成按时间", "给显示器加了一个支架"],
    ));
    pool.push(stale_relevant(
        "loadtest-vs-prod",
        "压测结果与线上表现不符",
        "压测结果与线上表现不符：先对齐数据规模与缓存命中率，再对齐实例数与网络路径",
        &["把书桌上的书按高度排列", "更换了手机的充电线"],
    ));
    pool.push(stale_relevant(
        "dashboard-empty",
        "监控大盘没有数据",
        "监控大盘没有数据：先查采集端是否在推、再看查询窗口的时区与标签是否对得上",
        &["把投影仪的梯形校正调正", "整理了冰箱里的保鲜盒"],
    ));
    pool.push(recency_fallback(
        "silence-after-alert-mute",
        "告警静默之后没人跟进",
        &["把窗户的密封条换新", "给自行车链条上了油", "整理了一下工具箱的螺丝"],
    ));
    pool.push(recency_fallback(
        "unrelated-astronomy",
        "星系自转曲线的观测证据",
        &["把花盆挪到了阳台", "换了一台除湿机"],
    ));
    pool.push(recency_fallback(
        "unrelated-cooking",
        "面团发酵总是发不起来",
        &["给台灯换了个开关", "把地毯收起来清洗"],
    ));
    pool.push(recency_fallback(
        "unrelated-music",
        "吉他和弦转换不顺畅",
        &["把书架的一层清空", "给键盘换了腕托"],
    ));
    pool.push(recency_fallback(
        "unrelated-gardening",
        "多肉植物叶片发软",
        &["整理了充电器和数据线", "把窗帘杆调高一点"],
    ));

    v.extend(pool);
    v
}

// ───────────────────────── 评测主体 ─────────────────────────

/// 一条用例在一个召回口径下的 hit@k（命中 = 期望那条出现在前 k 个里）。
fn hit_at_k(got: &[usize], c: &Case) -> f64 {
    if got.contains(&c.expect) {
        1.0
    } else {
        0.0
    }
}

/// 整个用例集在一个召回口径下的 recall@k（平均值）。
fn recall_at_k(recall: impl Fn(&[Memory], &str, usize) -> Vec<usize>, cs: &[Case]) -> f64 {
    let sum: f64 = cs.iter().map(|c| hit_at_k(&recall(&c.memories, c.query, c.k), c)).sum();
    sum / cs.len() as f64
}

// ───────────────────────── pass^k（稳定性口径）─────────────────────────

/// 把一条用例搬到**另一个池**上：期望条目按**内容**找回新下标。
///
/// 变体会改变池序与池长，`expect` 是下标而不是身份——所以每次搬池都必须重算，
/// 否则「变体」测的就成了「下标在搬池后还对不对」，与召回无关。
/// 内容重复时取首个（`variants` 里的重复变体正是靠这一点对齐）。
fn rebuilt(c: &Case, memories: Vec<Memory>, target: &str) -> Case {
    let expect = memories
        .iter()
        .position(|m| m.content == target)
        .unwrap_or_else(|| panic!("变体池里找不回期望条目（内容被改动了）：{}", c.name));
    Case { name: c.name, query: c.query, memories, expect, k: c.k }
}

/// 一条用例的**合法表示变体**。
///
/// ## 为什么变体不是「同一份输入跑四遍」
///
/// 排序内核是确定性函数：同一输入跑四遍结果逐位相同，`pass^4 ≡ pass@1`。
/// 拿这种「重复」当 pass^k 是公式装样子——它一个信息也不增加。
///
/// 真实世界里变的是**输入的外观**，不是随机种子：
///   - 池的物理顺序（读侧拿到的顺序不由本模块决定）；
///   - 池里还有更旧的无关条目（记忆只增不减，池会一直变长变旧）；
///   - 同一条记忆被存了两次（巩固/重放会重复入池，产生同 ts 平手）。
///
/// 这三者都是「答案不该变」的不变量。**一条用例只有在全部变体下都命中，才记 1 分。**
/// 于是 `pass^k`（k = 变体数）比 `pass@1` 严格：它要求命中不依赖某一种池外观。
fn variants(c: &Case) -> Vec<Case> {
    let target = c.memories[c.expect].content.clone();
    let mut out = vec![c.clone()];

    // V1 · 池序反转：输入物理序不该影响名次。
    let mut reversed = c.memories.clone();
    reversed.reverse();
    out.push(rebuilt(c, reversed, &target));

    // V2 · 追加一条**更旧**且与查询零重叠的条目。
    //     噪声用拉丁词而非中文：中文任意两段都可能共享二元组，"零重叠"无法保证，
    //     而这条变体要测的正是「无关的旧条目不该被顶上来」，噪声必须真的无关。
    let mut older = c.memories.clone();
    let min_ts = older.iter().map(|m| m.ts).min().unwrap_or(0);
    older.push(Memory {
        content: "zzz qqq xxx".to_string(),
        tag: "经验",
        ts: min_ts.saturating_sub(500),
    });
    out.push(rebuilt(c, older, &target));

    // V3 · 每条出现两次：幂等（重复入池不该改变第 1 名）+ 同 ts 平手走 seq。
    let mut duped = Vec::with_capacity(c.memories.len() * 2);
    for m in &c.memories {
        duped.push(m.clone());
        duped.push(m.clone());
    }
    out.push(rebuilt(c, duped, &target));

    out
}

/// 整个用例集在一个召回口径下的 `pass^k`：**全部变体都命中**的用例占比。
///
/// 返回 `(分数, [(用例名, 未通过的变体序号)])`——分母固定为用例数 n，不是变体
/// 总数：一条用例挂在一个变体上就是没通过，不该被另外三个变体的成功摊薄。
///
/// ## 命中按**内容**判，不按下标判
///
/// 变体会改池长，下标跟着变；更关键的是「重复入池」变体里同一条记忆有两行，
/// 而排序的第三键是 **seq 降序**——seq 更大的那一份赢得平手。此时按下标判
/// 命中会把**合法的命中**记成失败（拿到的正是用户想找的那条，只是另一行）。
/// 召回要的是「哪条记忆」，不是「哪个下标」。
fn pass_at_k(
    recall: impl Fn(&[Memory], &str, usize) -> Vec<usize>,
    cs: &[Case],
) -> (f64, Vec<(&'static str, Vec<usize>)>) {
    let mut failed = Vec::new();
    for c in cs {
        let target = c.memories[c.expect].content.as_str();
        let mut misses = Vec::new();
        for (i, v) in variants(c).iter().enumerate() {
            let got = recall(&v.memories, v.query, v.k);
            let hit = got.iter().any(|&j| v.memories[j].content == target);
            if !hit {
                misses.push(i);
            }
        }
        if !misses.is_empty() {
            failed.push((c.name, misses));
        }
    }
    ((cs.len() - failed.len()) as f64 / cs.len() as f64, failed)
}

// ───────────────────── 换述集（同义改写的查询）─────────────────────

/// 换述集：`fixtures/paraphrases.tsv`，33 条用例各 ≥2 种说法。
///
/// 这份表是**冻结数据**：改它要同步改 [`PARAPHRASES_FINGERPRINT`]，并在
/// `fixtures/README.md` 写为什么改（那才是下一次改它的人会看的地方）。
const PARAPHRASES_TSV: &str = include_str!("fixtures/paraphrases.tsv");

/// 换述集的指纹（FNV-1a 64，按 `\r\n → \n` 归一后算字节）。
///
/// **为什么是这个算法**：本程序是裸 `rustc` 编译的单文件，没有 Cargo 依赖可用；
/// 这条锁只需要「改一个字节就变」，密码学强度不是目的。目的是让「把换述改得
/// 更容易命中」在门禁里变成一道**显式的红**，而不是几行措辞 diff。
const PARAPHRASES_FINGERPRINT: u64 = 0x5fe9_a03f_3f96_1ae5;

/// FNV-1a 64：常量乘积、逐字节异或，八行就够，不引依赖。
fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 逐行读换述表：`用例名<TAB>换述`，`#` 与空行忽略。
///
/// 坏行**直接 panic** 而不是跳过：静默跳过等于「少了一条换述但读数照出」，
/// 而「每案 ≥2 条」正是这份夹具的全部意义（少一条，样本量悄悄变小）。
fn paraphrase_rows() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for (i, line) in PARAPHRASES_TSV.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, para) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("换述表第 {} 行不是 `用例名<TAB>换述`：{line}", i + 1));
        assert!(
            !para.trim().is_empty(),
            "换述表第 {} 行的换述是空的（空查询会退化成纯 ts 序，读数就没有意义）",
            i + 1
        );
        out.push((name, para));
    }
    out
}

/// 把换述挂到用例上：返回与 `cs` 等长的「该用例的换述列表」。
///
/// 用例名是两边的**唯一**关联键，所以两个方向都要判：表里的名字必须在
/// `cases()` 里存在（孤儿 = 写错名字，那条换述被静默丢掉），`cases()` 里
/// 每条必须有 ≥2 条换述（漏写 = 新加用例时忘了配题）。
fn paraphrase_index(cs: &[Case]) -> Vec<Vec<&'static str>> {
    let mut out = vec![Vec::new(); cs.len()];
    for (name, para) in paraphrase_rows() {
        let i = cs.iter().position(|c| c.name == name).unwrap_or_else(|| {
            panic!("换述表里的用例名不在 cases() 里：`{name}`——名字写错即静默丢一条换述")
        });
        out[i].push(para);
    }
    for (c, ps) in cs.iter().zip(&out) {
        assert!(
            ps.len() >= 2,
            "用例 `{}` 只有 {} 条换述（要求 ≥2）：一条换述只能代表「一种说法」，\
             测不出「换个说法」这件事的稳定性",
            c.name,
            ps.len()
        );
    }
    out
}

/// 一条用例的换述命中率（命中条数 / 换述条数）。
///
/// 分母是该用例自己的换述条数而不是全局常数：换述条数将来可能不等长，
/// 用全局常数会在那一刻把某些用例的权重算错，而错法不显眼。
fn paraphrase_hit_rate(c: &Case, ps: &[&str]) -> f64 {
    let hits = ps
        .iter()
        .filter(|&&q| hit_at_k(&production_recall(&c.memories, q, c.k), c) > 0.0)
        .count();
    hits as f64 / ps.len() as f64
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// 两串词集的 Jaccard 重叠——「换述与原述的字面有多近」的读数。
///
/// 它是纪律②（避开原述二元组）的唯一可见证据：换述越像原述，这个数越高，
/// 而依赖度就越低——**低得不一定是排序好，可能是题出得太像**。
fn jaccard(a: &[String], b: &[String]) -> f64 {
    let sa: std::collections::BTreeSet<&String> = a.iter().collect();
    let sb: std::collections::BTreeSet<&String> = b.iter().collect();
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn main() {
    let cs = cases();
    println!("召回评测集：{} 条用例\n", cs.len());
    for c in &cs {
        println!(
            "  · {}\n      查询：{}\n      期望：第 {} 条（k={}）",
            c.name, c.query, c.expect, c.k
        );
    }

    // 两个口径跑的是**同一份生产排序内核**（recall_score）：
    //   production = `rank(entries, Some(query))` —— 有查询（生产读侧口径）；
    //   trivial    = `rank(entries, None)`       —— 无查询（J2 平凡值，ts 序）。
    let prod = recall_at_k(production_recall, &cs);
    let triv = recall_at_k(trivial_recall, &cs);
    let (pass_all, pass_failed) = pass_at_k(production_recall, &cs);
    println!("\n生产投影（recall_score）的 recall@k：");
    println!("  {:<14} {:.3}  ← 内容匹配（有查询，生产读侧口径）", "production", prod);
    println!("  {:<14} {:.3}  ← 纯 ts 降序（无查询，J2 平凡值）", "trivial", triv);
    println!("  {:<14} {:.3}  ← pass^4：池序反转 / 更旧噪声 / 重复入池 三变体全过", "pass^k", pass_all);
    for (name, misses) in &pass_failed {
        let tags: Vec<String> = misses.iter().map(|i| format!("V{i}")).collect();
        println!("      ✗ 未通过变体：{name} @ {}", tags.join(","));
    }

    // ── 第四个数字：换述召回率与依赖度──────────────────────────
    //
    // 池不动，只把查询换成同义换述。`para` = 换述命中率；`dep = prod − para`
    // 是**依赖度**（越大越靠字面），它才是「抽象层级」那一格要的效标。
    let ps = paraphrase_index(&cs);
    let per_case: Vec<f64> = cs
        .iter()
        .zip(&ps)
        .map(|(c, p)| paraphrase_hit_rate(c, p))
        .collect();
    let para = mean(&per_case);
    let dep = prod - para;

    // 「措辞能左右结果」的用例：**时间序兜底会漏**的那些。
    // 分类用可执行判据，不加人工标签——trivial（无查询 ⇒ 纯 ts 降序）漏掉的
    // 用例才可能因换个说法改变结果；trivial 也命中的用例（期望的就是最新那条）
    // 任何查询都按时间兜底命中，换述读数在上面**结构上恒为 1**。
    let content_decided: Vec<usize> = (0..cs.len())
        .filter(|&i| {
            let c = &cs[i];
            hit_at_k(&trivial_recall(&c.memories, c.query, c.k), c) == 0.0
        })
        .collect();
    let para_c = mean(
        &content_decided
            .iter()
            .map(|&i| per_case[i])
            .collect::<Vec<_>>(),
    );
    let prod_c = mean(
        &content_decided
            .iter()
            .map(|&i| {
                let c = &cs[i];
                hit_at_k(&production_recall(&c.memories, c.query, c.k), c)
            })
            .collect::<Vec<_>>(),
    );
    let dep_c = prod_c - para_c;

    println!("\n换述读数（同一批用例、同一个池，只把查询换成另一种说法）：");
    println!(
        "  {:<14} {:.3}  ← 换述召回率（{} 条换述 / {} 案）",
        "paraphrase",
        para,
        paraphrase_rows().len(),
        cs.len()
    );
    println!(
        "  {:<14} {:.3}  ← 依赖度 = 原述 − 换述（越大越依赖字面）",
        "dependency", dep
    );
    println!(
        "  {:<14} {:.3}  ← 同一差值，只算措辞能左右结果的 {} 案（另 {} 案时间兜底恒命中）",
        "  └ 子集",
        dep_c,
        content_decided.len(),
        cs.len() - content_decided.len()
    );
    for (c, r) in cs.iter().zip(&per_case) {
        if *r < 1.0 {
            println!("      · 换述未全中：{}（{:.2}）", c.name, r);
        }
    }

    // ── 判据 ────────────────────────────────────────────────────────────
    //
    // 0. **n 自己是判据的一部分**。全过只能说出「失败率上界 = 3/n」
    //    （规则三：n 次试验 0 失败 ⇒ 95% 置信下失败率上界 3/n）。n=3 时
    //    这个上界是 100%，即「三条全过」什么也没说；n≥32 才让 1−3/n>0.90
    //    成为一句能被引用的陈述。用例数是**样本量**，不是体量装饰。
    let n = cs.len();
    let bound = 3.0 / n as f64;
    assert!(
        n >= 32,
        "用例数 n 是判据的一部分：全过只能声称成功率 ≥ 1−3/n；n={n} 时上界 {bound:.3} 说不出 >0.90"
    );
    assert!(
        prod <= 1.0 && triv <= 1.0,
        "recall@k 是比例，不可能超过 1"
    );

    // 1. **语义维必须真的在算**：生产口径（有查询）严格优于平凡值（无查询）。
    //    否则 recall_score 里的内容分支是死的——「换汤不换药」的静默失效。
    assert!(
        prod > triv,
        "内容匹配必须严格优于纯时间序：production={prod} trivial={triv}"
    );

    // 2. **命中不依赖池的一种外观**（pass^k）：池序反转 / 更旧噪声 / 重复入池，
    //    三变体全过才算过。`pass^k ≤ pass@1` 是结构恒等式（V0 就是原样）——
    //    反过来成立说明变体集没真的扰动输入，这条恒等式即是它的空转探测。
    let vs = variants(&cs[0]);
    assert_eq!(vs.len(), 4, "变体集固定为 原样 + 三种扰动");
    assert_ne!(
        vs[1].memories[0].content, vs[0].memories[0].content,
        "V1 必须真的反转池序（否则这条扰动是空转）"
    );
    assert_eq!(
        vs[2].memories.len(),
        vs[0].memories.len() + 1,
        "V2 必须真的多一条更旧噪声"
    );
    assert_eq!(
        vs[3].memories.len(),
        2 * vs[0].memories.len(),
        "V3 必须真的把每条重复一次"
    );
    assert!(
        pass_all <= prod + 1e-9,
        "pass^k 不可能高于 pass@1（V0 即原样）：pass^k={pass_all} prod={prod}"
    );
    assert!(
        pass_all >= 0.90,
        "稳定性低于 0.90：命中依赖池外观，未通过 {pass_failed:?}"
    );

    // 2b. **失败通道是活的**：把 expect 指到一条与查询零重叠的旧记忆上，
    //     `pass_at_k` 必须报出失败。一条永远为真的计数器与没有计数器同形。
    let mut bad = cs[0].clone();
    bad.expect = bad.memories.len() - 1; // 「今天改了 README」——最无关也最新
    let (bad_score, bad_failed) = pass_at_k(production_recall, std::slice::from_ref(&bad));
    assert!(
        bad_score < 1.0 && !bad_failed.is_empty(),
        "故意指错 expect 时 pass^k 必须下降：score={bad_score} failed={bad_failed:?}"
    );

    // 3. **每条用例至少被一个口径命中**：`expect` 下标若写错（越界 / 指向
    //    错的格子），这里会立刻暴露——全套命中数是 0 就意味着集合本身失效。
    let any_hit = cs.iter().any(|c| {
        hit_at_k(&production_recall(&c.memories, c.query, c.k), c) > 0.0
            || hit_at_k(&trivial_recall(&c.memories, c.query, c.k), c) > 0.0
    });
    assert!(any_hit, "用例集必须可命中：expect 下标全错时此断言红");

    // 4. **反例**：把第一条用例的 expect 改成**一个明确排不进前 k 的下标**，
    //    生产口径的命中数必须下降。这一条防的是「整套集合恒真」。
    //
    //    ⚠️ 陷阱：只改 expect、留着原来的 k，反向可能**不反向**——若该用例
    //    k=2 而池里只有 3 条，改后的下标照样落在前 2 里，分数一个不动
    //    （实测 1.0 → 1.0，等于多了一条恒真断言）。所以反向必须**同时**
    //    把 k 收紧到 1，让被改指的那条必然落在窗口外。
    let mut cs2 = cs.clone();
    cs2[0].expect = 1; // 「今天改了 README」——与查询零重叠
    cs2[0].k = 1; // 收紧窗口：内容口径此时只会把第 0 条放进来
    let rev = recall_at_k(production_recall, &cs2);
    assert!(
        rev < prod,
        "改错 expect 必须让分数下降：原 {prod} → 改后 {rev}"
    );
    // 且下降的量正好是 1/|用例集|（只动了一条）——多降少降都说明改动不止一处生效
    let one_step = 1.0 / cs.len() as f64;
    assert!(
        (prod - rev - one_step).abs() < 1e-9,
        "只改了一条 ⇒ 只该掉一格分数：应掉 {one_step}，实掉 {}",
        prod - rev
    );

    // 5. **分词在真的切**：中文查询按字级二元组切分（拉丁词整词）——这是
    //    内容维度的地基。第一版按空白切，中文查询整成一个词，重叠数恒 0/1，
    //    内容口径**恒等于时间口径**（实测 0.333 = 0.333）。
    let t = recall_score::terms("用 heaptrack 定位内存泄漏");
    assert!(
        t.contains(&"heaptrack".to_string()) && t.contains(&"定位".to_string()),
        "拉丁词整词 + CJK 二元组：{t:?}"
    );
    assert!(
        !t.contains(&"用 heaptrack 定位内存泄漏".to_string()),
        "不许整句当一个词（那正是第一版恒等的死因）"
    );
    let t2 = recall_score::terms("内存泄漏怎么定位");
    assert!(
        t2.contains(&"内存".to_string()) && t2.contains(&"泄漏".to_string()),
        "纯中文段全部走二元组：{t2:?}"
    );

    // 6. **反向**：清空查询词 ⇒ 内核退化为 ts 降序（与平凡值同序）。
    let empty_terms = recall_score::terms("");
    assert!(empty_terms.is_empty(), "空查询无词可切");
    let tied: Vec<ScoreEntry> = (0..3)
        .map(|i| ScoreEntry {
            content: format!("同文 {i}"),
            tag: "经验".to_string(),
            ts: 100 + i as i64,
            seq: i,
        })
        .collect();
    let by_empty = rank(tied.clone(), Some(""));
    assert_eq!(
        by_empty.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![2, 1, 0],
        "空查询 ⇒ ts 降序（seq 无同 ts 冲突时即输入序的逆）"
    );

    // 7. **换述集是冻结数据**：改一个字节就红。
    //
    //    锁的不是「不许改」，而是「改要有痕迹」：换述写得越接近原述，依赖度越小、
    //    读数越好看，而那种改法在 diff 里只是几行措辞。指纹把这类改动变成一道
    //    显式的红，逼改动者在这里写理由。
    assert_eq!(
        fnv1a64(&PARAPHRASES_TSV.replace("\r\n", "\n")),
        PARAPHRASES_FINGERPRINT,
        "换述表被改过：同步改 PARAPHRASES_FINGERPRINT，并在 fixtures/README.md 的\
         「改动记录」里写为什么——换述是人写的，改它就能改读数"
    );

    // 7b. **每条换述必须真的换了措辞**（纪律①②的可执行那一半；另一半「不许看
    //     正文」只能靠人守，故写进 fixtures 头注释与 README）。
    let mut overlap_sum = 0.0;
    let mut overlap_n = 0usize;
    for (c, ps) in cs.iter().zip(&ps) {
        let q = recall_score::terms(c.query);
        for p in ps {
            assert_ne!(*p, c.query, "换述与原述逐字相同（用例 `{}`）：那不是换述", c.name);
            let t = recall_score::terms(p);
            assert_ne!(
                t, q,
                "换述 `{p}` 与原述的分词结果一模一样（用例 `{}`）：换了标点不算换说法",
                c.name
            );
            assert!(
                t.iter().any(|w| !q.contains(w)),
                "换述 `{p}` 的每个词都来自原述（用例 `{}`）：这不测「依赖字面」，只测「同义词也认得」",
                c.name
            );
            overlap_sum += jaccard(&t, &q);
            overlap_n += 1;
        }
    }
    let overlap_mean = overlap_sum / overlap_n as f64;

    // 8. **依赖度的定义域**：换述召回率不可能高于原述（原述 1.000 时这是恒等式，
    //    原述掉下来时它是真判据）。这一条防的是「某一侧算错方向」——比如把
    //    `dep` 写成 `para - prod`，那样读数看起来更"好看"，含义却反了。
    assert!(
        dep >= -1e-9,
        "依赖度 = 原述 − 换述，不可能为负：prod={prod} para={para}"
    );

    // 9. **每一条换述都是承重的**（这条读数的「能红」判据）。
    //
    //    把**任意一条**换述换成与原述无关的问句，依赖度必须精确上抬 1/(n·m)
    //    （m = 该用例的换述条数）；若该条本来就没命中，读数必须**不动**。
    //    两种都断言——只断言「变大」的话，一条永远不命中的换述（等价于无关问句）
    //    也能让集合看起来是满的，而它对读数一个字节的贡献都没有。
    //
    //    ⚠️ 只在「措辞能左右结果」的用例上做：时间兜底那一类换成什么问句都命中，
    //    灵敏度在那里恒为 0——那是第 10 条要单独证明的事，不是这里的漏测。
    const POISON: &str = "zzz 星际尘埃的谱线宽度";
    let mut checked = 0usize;
    for &i in &content_decided {
        let hits: Vec<bool> = ps[i]
            .iter()
            .map(|q| hit_at_k(&production_recall(&cs[i].memories, q, cs[i].k), &cs[i]) > 0.0)
            .collect();
        for j in 0..ps[i].len() {
            let mut alt = ps.clone();
            alt[i][j] = POISON;
            let rates: Vec<f64> = cs
                .iter()
                .zip(&alt)
                .map(|(c, p)| paraphrase_hit_rate(c, p))
                .collect();
            let got = prod - mean(&rates);
            let want = if hits[j] {
                one_step / ps[i].len() as f64
            } else {
                0.0
            };
            assert!(
                (got - dep - want).abs() < 1e-9,
                "用例 `{}` 的第 {} 条换述换成无关问句后，依赖度应上升 {want:.4}，实升 {:.4}\
                 ——读数没跟着动，说明这条换述不承重",
                cs[i].name,
                j,
                got - dep
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 32,
        "灵敏度必须在足够多的换述上验过（现在只验了 {checked} 条）：\
         验得太少时「每条都承重」随时可以退化成「恰好没验到坏的那条」"
    );

    // 10. **边界是算出来的，不是写出来的**：时间兜底那一类用例上，把换述换成
    //     无关问句，读数**一分不动**。这条同时说明两件事——它们只能进分母，
    //     不能进依赖度的叙述；以及第 9 条的灵敏度不是「任何用例都灵敏」。
    let blind: Vec<usize> = (0..cs.len()).filter(|i| !content_decided.contains(i)).collect();
    assert!(
        !blind.is_empty(),
        "用例集里应当存在时间兜底类用例（没有的话第 10 条与第 9 条是同一条，边界就没被证明）"
    );
    for &i in &blind {
        let mut alt = ps.clone();
        alt[i][0] = POISON;
        let rates: Vec<f64> = cs
            .iter()
            .zip(&alt)
            .map(|(c, p)| paraphrase_hit_rate(c, p))
            .collect();
        assert!(
            ((prod - mean(&rates)) - dep).abs() < 1e-9,
            "用例 `{}` 属时间兜底类（期望的就是最新那条），任何查询都命中 ⇒ \
             换述读数在上面恒为 1，换掉不该影响依赖度",
            cs[i].name
        );
    }

    println!("\n通过：生产排序内核 recall@k = {prod:.3} > 平凡值 {triv:.3}，");
    println!("      且 expect 下标可被反向证伪（改错后 production 掉到 {rev:.3}）；");
    println!("      pass^4 = {pass_all:.3}（同一批用例换池外观后仍全过）。");
    println!("      换述 {para:.3}（依赖度 {dep:.3}；{checked} 条换述逐条验过承重），");
    println!(
        "      换述与原述的二元组重叠均值 {overlap_mean:.3}（越高说明题出得越像原述）。"
    );
    println!("\n规则三：n={n} 条全过 ⇒ 95% 置信下失败率上界 3/n ≤ {bound:.3}，");
    println!("        即成功率 ≥ {:.3}（>0.90 需要 n≥29，>0.95 需要 n≥59）。", 1.0 - bound);
}
