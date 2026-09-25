//! VDFS 词表：状态 / 基础类型 / 呈现扩展名 / 节点动作 / 字段名 / 调用级参数的线上取值常量。

// ==================== 状态取值 ====================

pub const VDFS_STATUS_ACTIVE: &str = "active";
pub const VDFS_STATUS_WORKING: &str = "working";
pub const VDFS_STATUS_DISABLED: &str = "disabled";
/// **以错误结束** —— 节点存在，但上一次运行失败了。
///
/// 与 [`VDFS_STATUS_ACTIVE`]（就绪）并列的一个**真实状态**，不是标志位：
/// 「会话上次失败了」= `status == failed`，而不是「状态 + `last_failed` 布尔」——
/// 后者要求读状态的人同时读两个字段，漏读一处就静默错。
///
/// 词面与消息层的 `MessageStatus::Failed` 一致：同一个概念在两类节点上不换词。
pub const VDFS_STATUS_FAILED: &str = "failed";
pub const VDFS_STATUS_UNKNOWN: &str = "unknown";
/// **无运行状态** —— 显式声明「本节点没有会变化的状态」。
///
/// 与 [`VDFS_STATUS_ACTIVE`]（就绪，一个**真实**状态）不同，本值表示**不适用**：
/// 静态资源（如设置分区）本来就没有「运行中 / 就绪」可言，给它画一个状态点
/// 只是噪音。列表据此**不渲染状态点**（见 `docs/design/vdfs-frontend.md` §4.2）。
///
/// 缺省仍是 `active`（见 [`default_status`]）：只有**显式**声明本值的节点才会
/// 失去状态点，因此这是「声明出来的无状态」，不是「忘了填」。
pub const VDFS_STATUS_NONE: &str = "";

/// 节点基础类型：目录
pub const VDFS_KIND_DIR: &str = "dir";
/// 节点基础类型：文件
pub const VDFS_KIND_FILE: &str = "file";

// ==================== 呈现扩展名（约定，宿主可自行扩展） ====================
//
// 节点 `ext` 是宿主选择详情呈现方式的键。VDFS 只透传、不解释；
// 以下是**约定俗成**的几个取值，宿主可自由增添自己的扩展名。
// （会话域的两个扩展名 `session` / `message` 属会话词表，
// 住在 `plugins/session/plugin/words.rs`，不在此登记。）

/// 定义驱动表单（呈现描述放 `node.schema`）
pub const VDFS_EXT_FORM: &str = "form";
/// 纯文本编辑器
pub const VDFS_EXT_TEXT: &str = "text";
/// JSON 编辑器
pub const VDFS_EXT_JSON: &str = "json";
/// Markdown 编辑器
pub const VDFS_EXT_MARKDOWN: &str = "md";
/// 文件树（目录节点的默认呈现）
pub const VDFS_EXT_DIR: &str = "dir";
/// 整包（zip）——**导入**用扩展名：内容是一整个资源目录的压缩包
pub const VDFS_EXT_ZIP: &str = "zip";

// ==================== 节点动作（约定） ====================

/// 节点动作标识：**连接测试**（`vdfs/action` 的 `action` 取值之一）。
///
/// 动作标识由 provider 自持，VDFS 只透传、不解释（与 `ext` 同构）。此处登记的
/// 是当前的内置约定：
///
/// - [`VDFS_ACTION_TEST`]「测试连接」——模型 / MCP 这类外部资源的连通性自检；
/// - [`VDFS_ACTION_EXPORT`]「导出」/ [`VDFS_ACTION_IMPORT`]「导入」——**一对逆向**
///   动作：导出把整目录资源打包成一个 zip 随 [`VdfsActionResult::data`] 返回，
///   导入把这样一个 zip 的字节写进目标地址。两者都是 provider 自持的动词，
///   与 [`VdfsRequest::Write`] 同走 [`VdfsContent::b64`] 二进制通道，区别只在
///   「谁发起、对哪个地址」：`write` 是通用写入，导入是**本目录的一种操作**；
/// - [`VDFS_ACTION_TRUNCATE`] / [`VDFS_ACTION_CLEAR`]——列表类资源的**区间删除**：
///   前者删「该条及其之后」，后者清空整个列表。
///
/// ## 为什么「导入 / 导出」是动作而不是 `write` / `read`
///
/// 它们是**整目录包**的搬运，不是某个节点内容的读写：导出的产物**不属于**被导出的
/// 目录（是它的快照），导入的输入也不属于目标目录（是别处的快照）。用 `read` /
/// `write` 表达就得让「地址」同时承担「谁的内容」和「打包哪棵子树」两种含义；
/// 动作把这件事交给 provider 自己解释，VDFS 只透传。
///
/// 反过来说，**它们也不该在 [`VdfsProvider`] 上另立接口**：导入是一次「对某个地址
/// 做什么」的操作，与 [`VDFS_ACTION_TEST`] / [`VDFS_ACTION_EXPORT`] 同类——占的
/// 是详情页的一条动作，而不是核心 trait 的一个方法。
///
/// ## 为什么「截断 / 清空」是动作而不是 `delete`
///
/// [`VdfsProvider::delete`] 的全局语义是「**这一个**节点没了」——它是**逐节点**
/// 的。拿它表达「删一个节点却删掉了它后面所有」会成为一条**没人能预期的默认
/// 行为**；而拿 `cascade: bool` 之类的
/// 附加位区分，则让「是哪种删除」变成两个字段必须一起读。动作是 provider 自持的
/// 动词，正好承载这类**集合操作**：VDFS 只透传，不解释。
///
/// ## 区间删除怎么通知消费端
///
/// **逐条**发移除帧——每条是「那个节点没了」的元数据（路径 + 状态，几十字节）。
/// 另一种形态是「列表目录整份重读」，但它会把**保留的**条目也重传一遍：对「删几条」
/// 这个动作，逐条通知是更便宜的。
///
/// 移除帧走 provider 自己的变更通道（与它的增 / 改变更同一条）：从机制看，被删的
/// 就是那一个条目——「删这一段」与「删这一个」在**单条**变更上完全同形，机制因此
/// 不需要认识「区间」这个概念。会话消息的落地形态见
/// `session/plugin/vdfs_provider.rs::truncate_messages` / `clear_messages`。
///
/// 回执里的被删 id 列表（随 [`VdfsActionResult::data`]）是**权威**列表：
/// 调用方据此幂等对齐本地视图，不依赖推送。
pub const VDFS_ACTION_TEST: &str = "test";
/// 节点动作标识：**导出**（打包下载；与 [`VDFS_ACTION_IMPORT`] 互为逆向）
pub const VDFS_ACTION_EXPORT: &str = "export";
/// 节点动作标识：**导入**（整包写入；与 [`VDFS_ACTION_EXPORT`] 互为逆向）。
///
/// 载荷是一个 zip 的字节（[`VdfsContent::b64`] 通道），provider 把它解释为
/// 「用这个包建出 / 覆盖本目录下的一份资源」——具体语义由 provider 自持。
/// 与 [`VdfsRequest::Write`] 的区别见本模块「节点动作」一节的说明。
pub const VDFS_ACTION_IMPORT: &str = "import";
/// 节点动作标识：**截断**（列表资源：删除该条目**及其之后**的全部条目）。
///
/// 结果里带被删条目的 id 列表（随 [`VdfsActionResult::data`]）——消费方用它做
/// 幂等对齐：本地若因锚点缺失而删窄了，据权威列表补齐。
pub const VDFS_ACTION_TRUNCATE: &str = "truncate";
/// 节点动作标识：**清空**（列表资源：保留容器本身，清掉全部条目）
pub const VDFS_ACTION_CLEAR: &str = "clear";
/// 节点动作标识：**中止**（停止该节点正在进行的活动）。
///
/// 与「截断 / 清空」不同，它不是集合操作，而是**对一个进行中活动的控制**：
/// 会话的 `<sid>` 上它表示「中止正在跑的那一轮」（`chat/abort` 的地址形态）。
///
/// ## 它作用的不是队列项
///
/// 有队列的资源（会话收件箱）上，「取消还没开始的」= 删那条队列项，
/// 「停止正在跑的」= 本动作。两者作用在两个不同的对象上，因此是两个动词，
/// 不合并成一个带开关的动词——合并之后「停了没有」取决于开关，机制就没法
/// 用地址回答「你停的是哪一个」。
pub const VDFS_ACTION_ABORT: &str = "abort";

// ==================== 插件容器的注册表动词 ====================
//
// 容器的根地址同时是两个视图的锚点：**资源树**（`List` 只给已挂载、且自己暴露了
// VDFS 的插件——那是「这里有哪些资源」）与**插件注册表**（`Action(PLUGINS)` 给
// 全部插件，含没有 VDFS 的与已停用的——那是「这个智能体由哪些插件组成」）。
//
// 两者刻意**不共用 `List`**：同一个地址在同一个操作上给两种答案，消费方就必须
// 先知道「我这次问的是哪个视图」才读得懂回包——那是把模式藏在参数里。注册表因此
// 是容器根的一个**自持动词**（`Action` 的本来用途，见本模块「节点动作」一节）。

/// 节点动作标识：**列出插件注册表**（容器根的自持动词）。
///
/// 回包见 [`VdfsActionResult::data`]：`{"plugins": [PluginEntry, …]}`。
pub const VDFS_ACTION_PLUGINS: &str = "plugins";
/// 节点动作标识：**启用**（把停用的插件恢复为已挂载）。
///
/// 作用于容器根下的**插件目录**（地址 = 插件名）。
pub const VDFS_ACTION_ENABLE: &str = "enable";
/// 节点动作标识：**停用**（插件连同配置与数据留在插件根，但不再挂载）。
///
/// 与 [`VDFS_ACTION_ENABLE`] 是一对：**都只作用于插件目录**，都不删除任何东西
/// （删除是 `Delete` 的语义）。必需插件也允许停用——「不可删」不等于「不可关」。
pub const VDFS_ACTION_DISABLE: &str = "disable";

/// [`VDFS_ACTION_PLUGINS`] 回包 `data` 的字段名：`{"plugins": [PluginEntry, …]}`。
///
/// 与动作标识分开命名，是因为它们是**两件事**：动作标识是「问什么」，字段名是
/// 「答成什么形状」。消费方（插件管理插件）读的是后者，因此两者必须各自有名字——
/// 拿动作名当字段名用，改名时就会把回包形状一起改掉。
pub const VDFS_PLUGINS_FIELD: &str = "plugins";

/// [`VDFS_ACTION_ENABLE`] / [`VDFS_ACTION_DISABLE`] 载荷里的字段名：**插件名**。
///
/// 名字走载荷而不是路径，是因为注册表动词作用在「注册表里的某一项」上，而那一项在
/// 资源树里可能根本没有位置——停用的插件不在资源树里（见 `composite/vdfs.rs` 的
/// 「资源树与插件注册表：两个视图」）。调用方（插件管理插件）从条目的地址拿到名字，
/// 放进载荷里交回容器。
pub const VDFS_PLUGIN_NAME_FIELD: &str = "name";

/// 容器根上 `Write`（= **安装一个插件**）的载荷字段名：`{"provider": "web"}`。
///
/// 它是「安装表单」的字段名（`DetailField.key`），由插件管理插件构造、由容器读取。
/// 不加 `VDFS_` 前缀是刻意的：它属于**表单字段**词汇（与 `DetailField` 的其它 key
/// 同域），不是 VDFS 协议词——前端持有它也不会进入协议镜像守卫的比对范围
/// （那组只认 `VDFS_*`）。
pub const PLUGIN_PROVIDER_FIELD: &str = "provider";

// ==================== 调用级参数（约定键名） ====================

/// 参数键：工作目录（本地文件子树解析相对路径的基准）。
///
/// 与宿主 ctx 的 `WORKDIR` 键同名同义：vdfs 访问层把请求 ctx 里的 workdir
/// 透传给 provider，使「相对路径从工作目录开始」这条既有本地地址规则
/// 在虚拟地址空间里保持不变。
pub const VDFS_PARAM_WORKDIR: &str = "workdir";

/// 参数键：有界列表的**条数上限**（`list`）。
///
/// 可选约定：provider 不认就当没传（全量），不认它的 provider 无需任何改动。
pub const VDFS_PARAM_LIMIT: &str = "limit";

/// 参数键：有界列表的**游标**——取该地址之前的一页（`list`）。
///
/// 游标是**地址**而不是页码：清单在两次请求之间会变（新增 / 删除 / 被顶到前面），
/// 偏移量会重复或漏项，地址不会。
pub const VDFS_PARAM_BEFORE: &str = "before";
