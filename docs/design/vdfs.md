# VDFS —— 虚拟动态文件系统规范

状态：现行规范（纯接口 + 统一文件系统 + 容器拓扑）
范围：Symbio 全部「资源与数据访问」能力（前端 + 大语言模型）
关联：
`symbio/src/symbio_core/vdfs_provider.rs`（**纯接口，权威定义：`VdfsProvider` trait
+ 域类型 + 路径工具**）、
`symbio/src/symbio_core/vdfs/host.rs`（symbio 桥：上下文注入 + 错误翻译）、
`symbio/src/plugins/vdfs/protocol.rs`（线路信封：`vdfs/*` 请求响应 + 协议路径）、
`symbio/src/plugins/vdfs/fs.rs`（**统一文件系统 `UnifiedFs`：`<根>` / 物理地址分流**）、
`symbio/src/plugins/vdfs/host.rs`（**访问层：拆信封 + 翻译操作 + 树遍历 + 事件投递**）、
`symbio/src/plugins/vdfs/physical.rs`（**物理层：磁盘真实文件 + 路径守卫**）、
`symbio/src/plugins/composite/vdfs.rs`（**拓扑：包含子目录列表的 provider**）、
`symbio/src/providers/vdfs_service/`（**宿主实现层：`VdfsProvider` 的三个集中实现**
——单文件 / 目录 / 内存，§11）

> 本文件只写**规范与机制**。具体子目录（会话、模型、设置分区、工作目录……）
> 一律属于**范例**（§13），不是机制的组成部分；子目录的增删改不影响本规范的
> 效力，机制演进也不以任何单个子目录为准。

---

## 1. 目标与不变量

**一个协议、一个页面机制、一处注册**——把「项目里所有的资源与数据」收敛为
同一个地址空间里的节点，让**前端与大语言模型用同一组操作、同一种寻址方式**访问：

```
                ┌────────────── 消费者 ──────────────┐
                │  前端（导航 / 列表 / 详情 / 表单）   │
                │  LLM（vdfs_* 工具）                 │
                └────────────────┬──────────────────┘
                                 │ 同一组 vdfs/* 操作 · 同一份载荷
                      ┌──────────▼──────────┐
                      │  vdfs 插件（访问层） │  拆信封 → 交给 UnifiedFs
                      └──────────┬──────────┘
                                 │ 同一个 UnifiedFs 实例
                      ┌──────────▼──────────┐
                      │ UnifiedFs（地址分流）│  `<根>` → 虚拟层；其余 → 物理层
                      └────┬───────────┬────┘
                           │           │
              ┌────────────▼──┐   ┌────▼──────────────┐
              │ 虚拟层         │   │ 物理层             │
              │ composite 组合 │   │ PhysicalFs（磁盘） │
              │ 视图           │   │ 工作目录相对地址    │
              └───────┬───────┘   └───────────────────┘
                      │ register_vdfs_provider(插件名, provider)
      ┌───────────────┬─────────────┼─────────────┬───────────────┐
 setting provider  session provider  model provider   …（各模块自持）
```

不变量（不可违反）：

1. **一个协议**：所有资源只用 `vdfs/*` 一组操作访问，**不得**新造私有资源协议。
2. **provider 自持**：数据操作、状态、校验、呈现描述全部由实现方负责；
   机制**只认访问位**（§4），不做任何按类型的特判。
3. **路径是唯一地址**：节点地址 = `<根>/<子目录>/<相对路径>`（虚拟）或
   `<工作目录相对地址>`（物理），无第二种寻址方式；判别只有一条——
   规范化后是否以 `<根>` 打头。
4. **接口与宿主解耦**：VDFS 接口定义**不依赖任何宿主类型**（§2）。
5. **同一集合**：LLM 能访问的资源集合与前端能访问的集合**恒等**——二者都经过
   同一个 `UnifiedFs`（§6），不存在「前端有而 LLM 没有」的资源。
6. **子目录名是使用方的概念**：provider **不知道也不提供自己在哪**；子目录名由
   注册方选定（约定 = 插件名）。
7. **拓扑归容器**：`<根>` 由**组合容器**（`composite`）的注册项服务；
   访问层、门面与 provider **都不持有**拓扑知识（§2.5）。
8. **没有「根级 provider」的概念**：composite 只是一个**恰好包含若干子目录的
   provider**——它可以被别的目录包含，子插件也可以是另一个 composite。它当前
   服务 `<根>`，只是装配安排（被登记进了单槽位），不是它的属性。

## 2. 分层与开放边界

### 2.1 centerpiece = 纯接口

VDFS 的核心是**一个包含全部资源操作的接口**——`VdfsProvider` trait。它**不是**
一组请求 / 响应结构；请求响应只是它的线路投影。

这与既有的 provider 组织方式一致（见 `symbio_core/model_provider.rs` 的模块文档）：
**core 只暴露纯 object-safe trait**，协议适配与线路形状留在插件内部。

```
symbio_core/vdfs_provider.rs   ← centerpiece：VdfsProvider trait
                                  + 域类型（VdfsNode / VdfsContent / VdfsAccess /
                                    VdfsWriteResponse / VdfsError / VdfsChange）
                                  + VdfsContext + 路径工具 + 回填
symbio_core/vdfs/host.rs       ← symbio 桥：上下文注入 + 错误翻译
plugins/vdfs/protocol.rs       ← 线路信封：vdfs/* 请求响应 + 协议路径常量
                                  + VdfsChangeEvent（总线事件形状）
plugins/vdfs/fs.rs             ← UnifiedFs：地址分流 + 口径映射 + 根守卫
plugins/vdfs/physical.rs       ← 物理层：磁盘文件 + FsPolicy 安全守卫
plugins/vdfs/host.rs           ← 访问层：拆信封 + 翻译操作 + 树遍历 + 事件投递
plugins/composite/vdfs.rs      ← 拓扑：逐子插件收集子目录 provider，组合成
                                  包含子目录列表的 provider
```

trait 上收拢全部操作（列 / 读 / 写 / 删 / 建 / 移 / 订阅），未实现者保持默认
（`NotImplemented`）。因此「一类资源 = 实现一个 trait」——新增资源不触碰线路层、
不触碰访问层，也不需要考虑自己被放在哪里（§2.4）。

### 2.2 分层与依赖

| 层 | 文件 | 允许依赖 | 说明 |
|---|---|---|---|
| 纯接口（centerpiece） | `symbio_core/vdfs_provider.rs` | `std` / `serde` / `serde_json` / `async_trait` | `VdfsProvider` trait、域类型、`VdfsContext`、路径工具、回填 |
| 宿主桥 | `symbio_core/vdfs/host.rs` | 宿主自有 | 上下文注入、`VdfsError` ↔ `PluginError` |
| 线路信封 | `plugins/vdfs/protocol.rs` | 上面两层 | `vdfs/*` 请求 / 响应、协议路径常量与 `VDFS_OPS`、`VdfsChangeEvent` |
| 地址分流 | `plugins/vdfs/fs.rs` | 上面两层 | `UnifiedFs`：`<根>` / 物理分流、口径映射、`normalize_addr` |
| 物理层 | `plugins/vdfs/physical.rs` | 上面两层 | 磁盘 IO + `FsPolicy`（白名单 / 符号链接 / 限额） |
| 访问层 | `plugins/vdfs/host.rs` | 宿主自有 | 拆信封、翻译操作、树状遍历、事件总线投递 |
| 拓扑 | `plugins/composite/vdfs.rs` | 宿主自有 | 逐子插件收集、组合成包含子目录列表的 provider |

硬约束：

- **纯接口层不得出现 `use crate::…`**。`vdfs_provider.rs` 可原样抽出为独立
  crate，供任何宿主（不限于 symbio）复用。
- **core 不暴露线路类型**：`VdfsPathRequest` / `VdfsListResponse` … 只存在于
  `plugins/vdfs/protocol.rs`，core 的任何 `pub` 面都不得出现它们。这是与
  `model_provider` 一致的分层（core = 纯 trait；协议适配在插件）。
- **trait 上不得出现「位置」相关成员**（`mount()` / `category()` 之类，§2.4）——
  provider 是纯目录语义，不知道也不关心自己出现在哪个地址。
- **宿主专有状态只能经 `VdfsContext` 注入**：它是一个不透明袋子
  （`Arc<dyn Any + Send + Sync>`），provider 用 `ctx.require::<T>()` 取回。
  新增宿主服务**不需要改动接口**。
- 错误类型由接口定义（`VdfsError`），不沿用任何宿主错误枚举；宿主在桥层
  做一次性翻译。
- 呈现描述（如表单定义）作为**不透明 JSON** 放在节点的 `schema` 字段里：
  VDFS 只透传、不解释，因此接入方可自由选择方言（JSON Schema、自有表单定义……）。

### 2.3 实现方为什么依赖 core 而不是插件

实现方（如 `setting` 插件）是**插件**而非宿主本身，它需要两件宿主能力：
从 `VdfsContext` 取回宿主句柄、把下游 `PluginError` 翻回 `VdfsError`。
这两件事放在 `symbio_core::vdfs::host`（桥层），使实现方**只依赖 core**，
不去依赖 `plugins/vdfs`——否则插件之间就产生了横向依赖。访问层（`dispatch`）
不是实现方需要的，因此留在 vdfs 插件内部。

### 2.4 子目录名不属于 provider

**「放在哪」是使用方的概念**：谁使用 provider，谁决定它在虚拟树上的位置。因此：

- trait 上**没有** `mount()` / `category()` / `root()` 之类的成员——provider
  既不管理也不提供自己的地址；
- 子目录名在注册时由注册方给出：`register_vdfs_provider(目录名, provider)`，
  **约定用插件名**（`PLUGIN_*` 常量）。插件名在宿主内唯一，天然合格，
  无需另立命名机制；
- provider 的方法只接收**自身子树内的相对路径**（`""` = 自身目录）；
  全路径 `<子目录>/<rel…>` 由使用方（容器）拼接与回填；
- 子目录节点由容器合成（`dir_node`：携带子 provider 的自述与访问位），
  provider 不参与；
- 变更事件（`VdfsChange`）同样不含子目录名——provider 只报子树内相对路径，
  容器补成树内全路径、门面再补成对外展示地址（§9）。

好处：provider 实现者不必先想一个名字、也不必担心重名；同一个实现可被放在
任何位置；换使用方策略（改名字、改顺序）不需要触碰任何 provider。

### 2.5 拓扑只归容器

「谁在哪个子目录」只由**组合容器**知道。容器实现 `VdfsProvider`，把自己聚合出的
视图注册到 `CapabilityVisitor` 的**单槽位**：

- **容器**（`composite`）＝ 唯一持有拓扑的角色：它逐个子插件收集注册项、组装成
  「包含子目录列表的 provider」，并 `register_vdfs_root(...)`；
- **门面**（`UnifiedFs`）＝ 只认地址前缀：`<根>` → 虚拟层（容器注册的视图），
  其余 → 物理层。它**不认识任何子目录清单**，因此「新增资源」不需要改门面；
- **访问层**（`vdfs` 插件）＝ 拆信封 + 取 `UnifiedFs` + 转发，同样没有拓扑知识；
- **provider**（各插件）＝ 只认自身子树内相对路径。

于是拓扑知识的落点**有且只有一处**（容器），且容器不依赖访问层、访问层与门面
不依赖任何具体资源——四者可以独立替换（§10.3）。

## 3. 地址与节点模型

### 3.1 地址

对外只有**一条判别规则**（`UnifiedFs::half_of`）：

```
<根>            根目录（系统资源清单）           ─┐
<根>/<子目录>/…  某插件子树                       ├─▶ 虚拟层（容器注册的视图）
                                                  ─┘
README.md        工作目录相对地址                 ─┐
src/main.rs      同上                              ├─▶ 物理层（磁盘真实文件）
D:/tmp/a.txt     绝对路径                          ─┘
```

- **规范化**（`normalize_addr`）折叠空段与 `.`、拒绝 `..` 穿越、去掉首部分隔符；
  **不强加前导 `/`**——虚拟地址以 `<根>` 打头、物理相对地址以名字打头、
  绝对物理地址以盘符打头，统一成「以 `/` 分隔的段序列」即可。
- **`<根>` 必须独占首段**：`<根>foo` 不算虚拟地址（按物理地址处理）。
- 两层**口径映射**只在门面进出两处各做一次：
  `"<根>/session/x"` → 虚拟层收到 `"session/x"`；结果回程补回 `<根>/` 前缀。
  物理层不需要映射。
- **树内相对路径是各 provider 的通用约定**：容器之下每个 provider 只见
  `""` / `<相对路径>`；core 不该知道宿主把根目录叫做 `<根>`。

#### 3.1.1 根名归属（全项目唯一一处字面量）

`<根>` **叫什么，是 vdfs 插件自己的挂载规则**，字面量只存在于
`plugins/vdfs/fs.rs::VDFS_ADDR_ROOT`（审计规则 S-010 强制）。它经
`AddrRootDecl` **静态声明**（`inventory::submit!`，编译期随二进制生效，
早于任何插件实例构造——容器装配子插件时就要用）。由此推出两条纪律：

1. **转发即改写（当前父地址）**：每次跨挂载边界的转发都在上下文里改写
   「当前父地址」（`VDFS_PARENT_ADDR`）：容器 `route` / `traverse` fork 子上下文时
   写入 `<根>/<子名>`；vdfs 协议派发跳（`CompositeVdfs::dispatch`）按
   `descend_addr` 续接。相对地址是常态——provider 全程只跟相对地址打交道。
2. **拼接走封装**：插件仅在少数**协议级**场合（提示词里印给模型的可编辑地址、
   错误指路）经 `absolute_addr(ctx, rel)` = 上下文父地址 + 相对地址 拼出绝对地址。
   不写死、不问全局。

前端同理不写死：启动期调 `vdfs/root` 拿根地址当**运行期数据**（前端锚点
`schemas/vdfsRoot`，`main.ts` 挂载前引导）。于是「改挂载名」= 改
`VDFS_ADDR_ROOT` 一行 + 全项目零改动。

### 3.2 节点

`VdfsNode` 是文件与目录的统一表达：

| 字段 | 语义 |
|---|---|
| `path` / `name` | 全路径（使用方回填） / 父内唯一标识（路径段） |
| `title` / `description` | 展示标题 / 语义说明（缺省 `title` = `name`） |
| `kind` | **场景标签**（自由取值；构造器缺省给 `dir` / `file`，场景可覆盖）；机制不据此判定 |
| `status` | `active` / `working` / `disabled` / `failed` / `unknown`；缺省 `active`，**空串 = 显式声明「无运行态」**（`VDFS_STATUS_NONE`，列表不画状态点）。词表权威处是 `symbio_core::vdfs_provider` 的 `VDFS_STATUS_*` <!-- vocab:VDFS_STATUS_ --> |
| `access` | 访问位（§4），**机制唯一的能力依据** |
| `ext` | 呈现扩展名——**前端据此选择详情页面**（§7） |
| `size` / `updated_at` / `children` / `binary` | 元数据 |
| `new_types` | 目录可接受的新建类型清单（§7 与 vdfs-frontend.md §5） |
| `schema` | 不透明呈现描述（宿主方言，VDFS 透传） |
| `attributes` | 场景扩展字段（flatten 到顶层） |

**目录与文件不做类型区分**：`access` 含 `l` 即可列（目录），含 `r` 即可读
（文件）。`is_dir()` 的实现就是 `access.list`。

**`hidden` 与文件系统的隐藏属性同义**，是**机制级的节点属性**：任何节点都可以带，
与它是不是目录、属于哪个场景无关——provider 的根**本来就是一个目录节点**，所以
「它在父目录的清单里显示还是隐藏」与文件 / 目录是同一件事。语义边界只有两条：

- **列表里不出现**：父目录的 `list` 结果不含它（`children` 计数同理）；
- **可达性不受影响**：按路径 `stat` / `read` / `write` / 子树操作一概照常。

因此「隐藏」既不是权限（那是 §4 的访问位），也不代表节点不在系统里——它照旧存在，
只是清单里不列。**过滤发生在产出列表的一方**：容器对自己合成的子目录清单、以及任何
子 provider 交回来的 `list` 结果都做同一条过滤，所以标了 `hidden` 的子节点不因来自
哪个 provider 而异。缺省不隐藏（序列化时省略）。

典型用法：内容只有一份配置文档、没有用户资源可浏览的目录
（`<根>/web` / `<根>/local` / `<根>/gateway`，见 §13.2）不必出现在导航列表里，
但它们仍按 `<根>/<插件>/PLUGIN.yml` 完全可寻址。

`kind` 与目录性**无关**：`dir` / `file` 只是构造器（`VdfsNode::dir` / `file`）给出的
**缺省场景标签**，场景想表达别的语义（插件名、条目类别…）就直接覆盖它。因此
`kind` 只有一个词表——「场景标签」，不存在「基础类型 + 场景类型」两套口径；目录性
永远只由 `l` 位表达。

**保留字段名**：`attributes` 会 flatten 到节点顶层，因此机制字段名是保留字，场景
扩展字段**不得**与之同名（否则扁平化后互相覆盖）：

```text
path  name  title  description  kind  status  access  ext
size  updated_at  children  binary  hidden  schema  new_types  attributes
```

新增机制字段时须同步本节；场景侧的命名建议带上自己的前缀（如 `config_type`、
`meta_tags`），但机制不做强制。

### 3.3 内容

`VdfsContent` 承载文本或二进制（互斥）：`text` 或 `b64`，由 `binary` 显式标注，
不做猜测；`size` / `mime` / `etag`（乐观并发令牌，可选实现）。

#### 写意图 `create` 与两种目标形态

`vdfs/write` 的 `path` 可以指向**两种东西**，实现方都必须考虑：

- **具名节点**（`<名字>` / `<父>/<名字>`）：常规的「写这个节点」；
- **目录自身**（`""` = 自身根，或任何指向目录的地址）：使用方**没有给名字**
  ——「新建一个，叫什么由你定」。**这是「新建」在机制上的形态**：使用方只说
  建在哪个目录，不说叫什么（名字是 provider 的私有知识）。因此**写目录自身不是
  错误**：provider 生成一个名字（id 归 provider）、落盘、并**在
  `VdfsWriteResponse.path` 里给出新节点的相对路径**——那是使用方唯一能拿到新地址
  的地方。不支持（该目录没有可新建的类型）则照常报错。

`create` 位 = **使用方的写意图**：

| 目标 | `create = false` | `create = true` |
|---|---|---|
| 已存在 | 覆盖（`created = false`） | 覆盖（`created = false`） |
| 不存在 · **具名节点** | 写入型资源**就地创建**（`created = true`）；「更新既有对象的字段」型语义可报 `NotFound` | **创建**（`created = true`） |
| 不存在 · **目录自身** | 报错（没有可覆盖的目标） | **创建**，名字由 provider 生成 |

⚠️ **具名 + 目标不存在时不要一律报 `NotFound`**：使用方要的是「给了名字就写得进去」
——配置型资源的地址**就是它的身份**（`model` / `mcp` / `skill` 皆如此），不存在就建
一个。只有「写的是某个**既有对象的一个字段**」（如会话 metadata）才该拒绝：没有对象
就没有可更新的字段。

于是「保存一份还没落盘的草稿」与「新建一项」是同一个动作的两种意图，使用方无需
先 `stat` 再决定写还是建（那会引入一次多余的往返与竞态）。

⚠️ **`create` 不改变内容的处理方式**：内容一律取自本次写入。**不要**把
`create = true` 实现成「忽略使用方给的内容、一律落默认值」——那会让「在详情页里
填好再保存」丢掉用户填的每一个字段。唯一例外是**内容为空**（「先建一个，随后再
填」，如新建会话）：此时由 provider 落一份自己的最小合法内容。

**二进制写入 = 整包导入**：目录型资源（skill / agent bundle…）以新建类型
`ext = zip` + `source = file` 声明「内容来自本地文件」，使用方选文件后走
`vdfs/write { create: true, b64 }`；provider 把它解释为**导入一个完整目录包**
（语义自持，VDFS 不解释）。因此导入不额外占一个操作（详见
[vdfs-frontend.md](vdfs-frontend.md) §5 与 §7 的 S12）。

**导出是导入的逆动作**：它不新增第二个操作，而是一个**节点动作**——
`vdfs/action { action: "export" }`，zip 随 `VdfsActionResult.data` 回传
（载荷 `{id, filename, b64}`，与 `VdfsContent.b64` 同构）。provider 不支持
时返回 `NotImplemented`，使用方据此不给出入口；支持与否由 provider 自陈，
与「新建类型」同理（详见 [vdfs-frontend.md](vdfs-frontend.md) §7 的 S13）。

### 3.4 插件配置 = 插件目录里的一个文件（`PLUGIN.yml`）

「一个插件的配置」就是一个**普通的可寻址文档**——既不设第二条配置协议，也不由
父插件代为存储。每个插件目录下都有一个 `PLUGIN.yml`，**谁写谁读**：

```
<根>/session/PLUGIN.yml     会话设置（与 `<根>/session/<会话 id>` 并列）
<根>/local/PLUGIN.yml       本地工具设置
<根>/gateway/PLUGIN.yml     开放接口设置
<根>/telegram/PLUGIN.yml    Telegram 设置
```

磁盘上就是 `<homedir>/<插件>/PLUGIN.yml`，与插件自己的数据**同处一个目录**
（`<homedir>/session/<会话 id>/`、`<homedir>/model/<id>/provider.json`…），
因此整个目录可以直接拷贝移植——搬走目录 = 搬走插件（连同配置与数据）。
系统级插件（`home` 与容器 `composite`）的目录是系统根本身，配置在
`<homedir>/PLUGIN.yml`。

- **地址形状**：`<插件目录>/PLUGIN.yml`——**真实文件名**，不是保留段。插件目录恒为
  目录（前端导航只列目录，§7），配置只是它下面的一个普通文件；插件自身的资源 id
  与它不冲突（会话 id 是 UUID、模型 / MCP 的 id 由 provider 生成或从地址末段派生，
  都带自己的前缀）。
- **节点形状**：`ext = form`、`access = rw`、`schema` = 该插件自己的
  `DetailDefinition`。`ext` 显式声明为 `form`，**覆盖**由文件名推导出的 `yml`——
  呈现方式由声明决定，不由文件名猜。读写用的就是 `vdfs/read` / `vdfs/write`（§5），
  与任何其它资源同一条链路、同一套寻址，因此前端与 LLM 用同一种方式改配置。
- **定义与校验同源**：提交值交给 `DetailDefinition::validate`，失败即字段级
  `VdfsValidationError`（§8）。字段定义由**配置的拥有者**产出（默认值从该插件的
  `Default` 读出），不另写一份 schema 字面量。
- **落盘就是写自己的文件**：`ConfigFile::apply` 的一条链是
  **校验 → 落内存 → 落自己的文件 → 广播**。没有 `save_config` 路由、没有切片推送、
  没有父插件参与——配置回到拥有者手上，父插件不认识任何子插件的配置。
- **身份字段是保留键**：`plugin_provider`（工厂 id）/ `plugin_name`（实例名）与配置
  字段同处一个文件，但**不参与配置反序列化**（`PluginDir` 读写时自动剥离 / 补回）。
  装配方据此判定「这个目录是不是一个可加载的插件」：文件存在、可解析、且
  `plugin_provider` 指向一个已注册的工厂（见 §13.2）。
- **副作用留在插件**：写完配置之后还要做什么（如网关重建监听）在插件自己的
  `write` 里做——`ConfigFile::apply` 返回后再执行，机制不引入回调抽象。
- **实现只写一次**：`symbio_core::plugin_dir` 的 `PluginDir`（目录 + 配置读写，
  含遗留字段清理）与 `ConfigFile`（节点形状 + 定义校验 + 落盘）——
  **值 + 一组函数**，不是 trait：没有注册表、没有回调。
- **凭据在配置里**：配置文件与其它节点一样受访问位约束（`r`），即**可读**。
  对外暴露面（如网关的只读白名单）需自行拒绝落在 `<插件目录>/PLUGIN.yml` 上的读取。
- **设置页只是「指路」**：设置插件的清单里会出现这些配置文档（见 §13.1），但条目
  携带的是**它自己的真实地址**（`<插件目录>/PLUGIN.yml`），读写仍走拥有者——同一份
  配置只有一个地址，设置页不代理读写、也不复制一份。

## 4. 访问位（r / w / l / t）

访问位是**机制唯一的能力依据**，线上表示为紧凑字符串（按 `r` `w` `l` `t` 顺序）：

| 位 | 语义 | 典型 |
|---|---|---|
| `r` | 读取内容 | 文件 |
| `w` | 写入内容 | 文件 |
| `l` | 列出直接子节点 | 目录 |
| `t` | 树状递归遍历 | 目录 |

- `"rl"` = 可读 + 可列；`"rw"` = 可读可写；`"wlt"` = 可写可列可遍历；`""` = 不可访问。
- 预置组合：`VdfsAccess::{READ, READ_WRITE, LIST, LIST_TRAVERSE, LIST_WRITE_TRAVERSE}`。
- **树状遍历 `t` 是声明式的**：provider 只需实现 `list` 并在目录节点上声明 `t`，
  访问层即可递归展开（`vdfs/tree`）；provider **不需要**实现遍历逻辑。
- 消费者（前端 / LLM）在动作前应依据访问位判定可做性，**不得**按 `kind` 猜测。

## 5. 协议操作

`plugins/vdfs/protocol.rs` 的 `VDFS_OPS` 是唯一操作清单（**线路信封只存在于插件内，
core 不暴露**）：

| 操作 | 请求载荷 | 响应 | 说明 |
|---|---|---|---|
| `vdfs/root` | `{}`（**不给地址**） | `VdfsListResponse` | **进入地址空间**：列出虚拟根，回包 `path` 即根地址（消费方当运行期数据持有，见 §3.1「根名归属」） |
| `vdfs/list` | `{path}` | `VdfsListResponse` | 列目录；`<根>` 返回系统子目录清单 |
| `vdfs/tree` | `{path, depth?, limit?}` | `VdfsTreeResponse` | 递归遍历（只下钻 `t` 位目录） |
| `vdfs/stat` | `{path}` | `VdfsNode` | 读元数据 |
| `vdfs/read` | `{path}` | `VdfsContent` | 读内容（`r`） |
| `vdfs/edit` | `{path, old_string, new_string?}` | `VdfsEditResponse` | 精确字符串替换（**访问层组合操作**：`read` → 替换 → `write`） |
| `vdfs/search` | `{path?, pattern}` | `VdfsSearchResult` | 文件名 Glob 搜索（**访问层组合操作**：递归 `list` + 过滤） |
| `vdfs/write` | `{path, text\|b64, create?, etag?}` | `VdfsWriteResponse` | 写内容（`w`） |
| `vdfs/delete` | `{path, recursive?}` | `VdfsDeleteResponse` | 删除 |
| `vdfs/mkdir` | `{path}` | `VdfsWriteResponse` | 新建目录 |
| `vdfs/move` | `{from, to}` | `VdfsMoveResponse` | 移动 / 重命名（同一半内） |
| `vdfs/watch` | `{path}` | `SuccessResponse` | 订阅该子树变更 |
| `vdfs/unwatch` | `{path}` | `SuccessResponse` | 取消订阅（与 watch 配对） |
| `vdfs/action` | `{path, action, payload?}` | `VdfsActionResponse` | 执行**节点动作**（provider 自持动词；未实现返回 `NotImplemented`） |

**每个操作都是对「统一文件系统」的一次方法调用**，访问层不含任何资源语义。
唯一由访问层自身实现的是 `vdfs/tree`——它按 `t` 位递归调用 `list`，属于
**机制级**通用能力，不是场景逻辑。（曾经还有 `vdfs/providers` 挂载清单端点，
已随 mount 概念退场删除——`<根>` 的 `list` 就是子目录清单。）

机制级守卫分两处（provider 都无需重复）：

**访问层统一施加**（`plugins/vdfs/host.rs`，与资源语义无关）：

- `..` 穿越 → `ValidationError`（在 `normalize_addr` 就发生）；
- `write` 必须携带 `text` 或 `b64`；
- 响应中的 `path` 缺省回填：provider 给出的是**子树内相对路径**（它自己为新条目生成的名字
  也只是 `<rel>`），容器**一律补成 `<子目录>/<rel>`**；只有空串（= 未填）才用请求地址兜底。
  ⚠️ 这条不能只对空串生效——`write` 的返回值是使用方得知「刚建出来的东西在哪」的**唯一**
  途径，漏补即等于新建之后找不到新节点（前端只能停在草稿上）。
- `ext` 缺省由 `name` 推导；`title` 缺省 = `name`。

**门面 / 容器统一施加**（`UnifiedFs` 与 `CompositeVdfs`，与资源语义无关）：

- 移动**跨半**（物理 ↔ 虚拟）在触达任何一层之前就被拒；
- 虚拟层内：自身目录与子目录根不可**读 / 删 / 移**，也不可 `mkdir`；
  ⚠️ **写不在其中**——写子目录根 = 写在**挂载点目录自身**上（§3.3「写目录自身」），
  容器一律转发给子 provider 判定，不做类型特判；
- `move` 跨子目录被拒（只允许同一子目录内）；
- 子节点 / 内容 / 写入响应的 `path` 回填为全路径（树内 → 展示口径）；
- 变更事件的相对路径补全为展示地址。

错误经 `VdfsError` 表达，桥层翻译为宿主错误（symbio → `PluginError`）。
`VdfsError::Invalid(VdfsValidationError)` 的结构化字段级错误序列化为 JSON 置于
错误文案位，前端可解析后逐字段高亮；解析失败按纯文本展示（向前兼容）。

## 6. 注册与收集

### 6.1 provider 注册（各插件 → 容器）

`CapabilityVisitor` 上的方法（均有默认实现，不破坏既有实现方）：

```rust
// 目录名 = 使用方选定（约定 = 插件名），provider 自身不含此概念
async fn register_vdfs_provider(&self, dir: &str, provider: Arc<dyn VdfsProvider>);
async fn list_vdfs_providers(&self) -> Vec<(String, Arc<dyn VdfsProvider>)>;  // order 升序
async fn get_vdfs_provider(&self, dir: &str) -> Option<Arc<dyn VdfsProvider>>;
```

插件在 `traverse` 的 `TRAVERSE_AVAILABLE_TOOLS` 分支里，**在注册工具的同一处**
顺带注册自己的资源，目录名用插件名：

```rust
if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
    // 工具…
    let me: Arc<dyn VdfsProvider> = self.clone();
    visitor.register_vdfs_provider(PLUGIN_SETTING, me).await;   // 插件名即子目录名
}
```

### 6.2 根登记（容器 → 系统）

`<根>` 的服务者是**单槽位**，当前由组合容器占据：

```rust
async fn register_vdfs_root(&self, provider: Arc<dyn VdfsProvider>);   // 重复注册覆盖
async fn get_vdfs_root(&self) -> Option<Arc<dyn VdfsProvider>>;        // 无容器时 None
```

**这不是「根级 provider」概念**：槽位只是装配点——谁被登记，谁的子树就成为
`<根>` 之下的内容。composite 可被别的目录包含、子插件也可以是另一个 composite；
登记本身不改变 composite 的任何行为与代码。`composite` 在**同一次**
`TRAVERSE_AVAILABLE_TOOLS` 广播里完成登记：

```rust
if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
    if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
        visitor.register_vdfs_root(self.vdfs.clone()).await;   // 装配安排
    }
}
```

### 6.3 组合视图：容器如何聚合子插件

容器**本身就是**「包含子目录列表的 provider」（`CompositeVdfs`，无任何中间结构）：
逐个子插件广播一次收集、每个子插件配一个**独立**收集器：

```rust
for child in children {
    let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
    let sub = host.fork();
    sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    sub.set(CAPABILITY_VISITOR, visitor.clone());
    child.clone().traverse(String::new(), sub).await?;
    dirs.extend(visitor.list_vdfs_providers().await);   // 必然属于该子插件
}
```

一次广播把整棵树收进同一个收集器，就无法区分某个 provider 是谁注册的；
**逐子插件 + 独立收集器**保证目录名天然归属于注册它的那个子插件。
**不缓存**——子插件集合由配置与生命周期决定，每次现取才与容器一致。

### 6.4 一次广播的语义，两条消费链路

**不新增第二条收集通道**。资源与工具、模型服务、系统提示词**共用同一次
能力广播**：

- **LLM 链路**：会话编排方 `collect_capabilities` → 广播 → 容器登记 `<根>`
  服务者、各插件注册 provider 进入 `CAPABILITY_VISITOR` → 工具执行时 `tool_ctx`
  携带该访问器 → `vdfs_*` 工具经 `root_of(visitor)` 取根，交给 `UnifiedFs`。
- **前端链路**：`vdfs` 插件收到 `vdfs/*` 请求后调 `resolve_fs(parent, ctx)`：
  `ctx` 无能力管理器时，它从父插件广播**同一次** `traverse(TRAVERSE_AVAILABLE_TOOLS)`，
  容器在广播中把视图注册进新的收集器，随即取回。

两条链路因此经过**同一个 `UnifiedFs`**（不变量 5）——不存在第二处地址规则。
取不到根（无组合容器）时虚拟层降级为**空目录**（`EmptyVdfs`）：`<根>` 可列出
但无内容，具体路径 `NotFound`；物理层与它无关，照常可用。于是「系统没有资源」
与「资源为空」表现一致，前端与 LLM 都不必特判。

### 6.5 上下文携带

大语言模型工具调用时，宿主在 ctx 中挂载 `CAPABILITY_VISITOR`
（会话侧 `attach_capabilities`，工具执行侧 `tool_ctx = ctx.fork()`）。
`vdfs_*` 工具据此取得"本次调用实际可用的地址空间"，因此**新增资源不需要
改动 vdfs 插件或工具集**。

上下文分两层：

| 层 | 形态 | 用途 |
|---|---|---|
| **宿主句柄** | `VdfsContext::host::<H>()`（类型化 downcast） | 宿主专有对象（如能力管理器） |
| **调用级参数** | `VdfsParams`（`serde_json::Map` + 约定键） | 运行时状态（如 `workdir`） |

调用级参数是**开放约定**而非类型化槽位：键名由使用方与 provider 用共享常量对齐
（`VDFS_PARAM_WORKDIR` = `"workdir"`），因此**新增一个约定参数不需要改接口**，
资源语义也不会渗进 `VdfsProvider`。翻译只在访问层发生一次
（`host::call_params`：宿主 ctx 的 `WORKDIR` → 约定键 `workdir`），provider 因此
**不认识宿主 ctx 的键名约定**。

`host::dispatch_with(fs, path, ctx, params)` 是唯一分发入口；不需要参数时传空的
`VdfsParams`。两条链路（前端协议入口 / LLM 工具）都经它，因此"前端能做什么"与
"LLM 能做什么"始终同源。

## 7. 前端机制：ext 决定详情页面

后端下发的 `ext`（扩展名）是**前端选择详情渲染器的唯一键**：

| `ext` | 渲染器 | 说明 |
|---|---|---|
| （目录，`l` 位） | 列表 / 树 | 中栏结构由访问位与 `t` 位决定 |
| `form` | 定义驱动表单 | 解析 `node.schema`（不透明 JSON，symbio 方言 = DetailDefinition） |
| `session` | 会话工作区 | 实时对话 |
| `md` / `json` / `text` | 文本类编辑器 | 内容经 `vdfs/read` 获取 |
| 其他 | 通用兜底 | 只读呈现 + 通用文本视图 |

约定：

- 扩展名**缺省由 `name` 推导**（`a.md` → `md`），provider 可显式覆盖
  （设置分区名无扩展名，显式声明 `form`）。
- **「新建」必须落到同一个渲染器**：新建类型的 `ext` 是**呈现扩展名**（地址末段
  后缀，provider 用 `id_of` 按它剥条目 id），它**不**决定详情怎么渲染——
  配置型资源（model / mcp / skill）落成后统一是 `ext = form`。因此
  `VdfsNewType` 另有两个字段描述**落成后的节点**：

  | 字段 | 语义 |
  |---|---|
  | `ext` | 呈现扩展名（地址后缀；`id_of` 剥 id 用） |
  | `node_ext` | 落成后的节点 `ext`（**渲染器键**）；缺省 = 与 `ext` 相同（会话即如此） |
  | `schema` | 落成后的节点 `schema`（`form` 渲染器所需的定义） |

  使用方据此在**还没创建**时就能渲染出该类型的详情页——草稿节点（无 id、无名字）
  用 `node_ext` 选渲染器、用 `schema` 出表单，于是「点新建」与「选中一项」进入的是
  **同一个详情页**，差别只在有没有内容。漏了 `node_ext` 会落到通用兜底，
  漏了 `schema` 会渲染出空表单。
- 前端只持有**纯 UI 映射**（ext → 组件、子目录名 → 图标），不硬编码资源类型、
  标签、能力或路径模板。分层：`schemas/vdfs.ts`（数据契约，零组件知识）→
  `registry/vdfsTypes.ts`（ext → 渲染器标识，零组件导入）→
  `registry/vdfsRenderers.ts`（标识 → 组件，唯一装配点）。
- 页面：`views/VdfsView.vue`（路由 `/vdfs/:dir(.*)*`，可选多级 `dir` 支持深链），
  页面逻辑全部在 `composables/useVdfs.ts`；左栏 = `<根>` 子目录导航（应用外壳
  承担）、中栏 = 当前目录、右栏 = 按 `ext` 分发的渲染器。**一个 VdfsView 对应一个
  vdfs 地址**：首页地址 = `<根>`（`/vdfs`），更深的地址都是 push 出来的页面
  （左上角返回键回上一级）——详见 [vdfs-frontend.md](vdfs-frontend.md)。
- 由此，整个前端逐渐收敛为「**通用资源状态查看 / 管理工具**」：
  新增一类资源只需后端新增一个 provider，前端零页面开发。

## 8. 校验职责

**校验属于 provider，不属于机制**：

- 机制只做与资源语义无关的守卫（§5）。
- provider 在 `write` 内完成必填 / 范围 / 枚举 / 类型 / 跨字段校验，失败返回
  `VdfsError::Invalid(VdfsValidationError { message, fields })`。
- **校验先于副作用**：校验不过时 provider 不得触达落盘或任何下游写入
  （配置文件的 `ConfigFile::apply` 即「先 `decode` 校验 → 再落内存 → 最后落自己的
  文件」；校验不过时内存与磁盘都不动）。
- 前端据 `fields[].field` 逐字段提示，与 `schema` 中的字段键对齐。

## 9. 实时性

- provider 在 `watch(path, sink)` 中开始监听，变化时调用 `sink(VdfsChange)`
  上报；`unwatch` 严格配对。
- **provider 报出的 `VdfsChange` 不含子目录名**：`path` / `to` 都是该 provider 子树
  内的**相对路径**（与 `list` / `stat` 同一坐标系）。位置是使用方的概念（§2.4），
  provider 无从得知。
- **`watch(path, sink)` 的 `path` 也是同一坐标系**（provider 根相对，`""` = 自身根）。
  provider 上报的路径**不相对被订阅的 `path` 收敛**——容器回填时只补**挂载名**
  （首段），不补被订阅的整条路径。两处若都做前缀处理就会重复拼接：

  ```text
  watch("session/abc/消息") → 容器拆出 dir="session"、rel="abc/消息"
  provider 报 "abc/消息/m1" → 容器补成 "session/abc/消息/m1"  ✓
  provider 若报 "m1"       → 容器补成 "session/m1"          ✗
  ```

  代价是订阅者会收到兄弟子树的变更（订阅一个会话会看到别的会话的事件），
  由消费者按路径前缀自行过滤——`list` / `stat` 亦然，坐标系因此始终只有一个。
- 消费能力由访问位与 provider 实现共同决定；默认 `watch` 为 no-op（无实时能力
  的 provider 直接成功，消费者退回手动拉取）。
- **订阅登记按 `kind` 全局持有，投递是同步的**（`vdfs::host::ChangeSubscriptions`）：
  每个自管变更源的 provider 持有一份表实例，`watch` 记一条 `(path, sink)`、
  `unwatch` 撤一条，变更点直接 `notify` 遍历投递——**没有中间广播通道、没有转发任务**。
  曾有 `broadcast::Sender` + 每次 `watch` spawn 一条转发任务的设计，已整体废除：
  它会在「无订阅者」时丢弃变更（前端尚未登记完的启动窗口正好落在这里），且任务
  生命周期要与 `unwatch` / 重连逐一配对，复杂度全部落在机制侧却换不到任何保证。
  同步投递的另一半收益是**顺序确定**：变更点返回即投递完成，不存在「已落盘但尚未
  转发」的中间态。
- **重叠订阅按「最具体者优先」恰好一次**：同一条变更若同时落在 `…/abc` 与
  `…/abc/消息/m1` 两条订阅之下，只投给路径更长的那条。这里的「一次」指的是
  **总线上的一次发布**——sink 的职责是把变更发进 `kind = "vdfs"` 频道，前端各消费者
  再按自己的作用域前缀过滤（订 `<根>/session` 的清单同样收得到 `…/session/abc/消息/m1`）。
  因此收敛为一条不会让任何人漏收，反而避免了同一变更被发布两次：转写的 `appended`
  若到两遍就是**叠字**。（表按 `kind` 全局持有，是因为同一 provider 每次 `traverse`
  都会新构造，按实例持有会让订阅与投递配不上对。）
- 宿主侧投递分两跳，**各补一次它那层才知道的信息**：
  1. **容器**（`CompositeVdfs::watch`）把 provider 的相对路径**补成树内全路径**
     （`<子目录>/<rel>`）后交给上层 sink；
  2. **门面**（`UnifiedFs`）把树内全路径**补成对外展示地址**
     （`<根>/…` 或物理地址），装进 `VdfsChangeEvent`，发布到全局事件总线
     （`kind = "vdfs"`，无会话关联、不入回放缓冲）。

  前端 `subscribe({ kind: 'vdfs' })` 按 `path` 前缀自行分流、防抖重拉。
- **`vdfs::host::notify_change(kind, path, change)` 只报三元组，不携带载荷**：它是
  provider 侧唯一的广播入口，带不带 `node` / `content` 由 provider 自己的 `watch`
  决定——**带载荷的增益投递只存在于 provider 自己实现的 `watch` 里**（下表 `created`
  / `updated` 的可选载荷即由此补上）。经 `notify_change` 进来的变更，消费者只能防抖重拉。
- **变更词汇**（`VdfsChange::change`，取值唯一，无场景自定义）：

  | 取值 | 语义 | 载荷 | 消费者动作 |
  |---|---|---|---|
  | `created` | 多了一个节点 | 可带 `node` / `content` | 列表插入一项（或重拉该目录） |
  | `updated` | 节点变了，**内容全量** | 可带 `node` / `content` | 就地替换（或重读该节点） |
  | `appended` | 节点**尾部多了 `delta`**，增量 | `delta` | 拼接 `delta`，**不重读** |
  | `deleted` | 节点没了 | — | 列表移除一项 |
  | `renamed` | 节点换了地址 | `to` | 改键（`to` = 新地址） |

  **载荷是「按变更类型可选」的，不是可有可无的装饰**：事件只说「哪里、怎么变」，
  「变成了什么」按类型附在载荷上——热路径窄、冷路径全。

  - `appended` 的载荷是 `delta`（增量文本），消费者**不得**借此触发重读——这正是
    它与 `updated` 的全部区别。它每帧都发，载荷必须保持只有增量：任何追加型数据
    （日志、转写、生成中的文档）若一律用 `updated` + 重读，流量是 O(n²)。
  - `created` / `updated` 每轮只有寥寥数次，provider **可以**把节点视图
    （`node`）与内容快照（`content`）一并带上，消费者因此无需 `stat` + `read`
    两个来回。载荷**可选**：不填时消费者回退到回读，因此这是纯增益扩展。
  - 逐字段重建 `VdfsChange` 是**错的**——转发层新增字段时会漏（且无编译错误）。
    使用方一律用 `VdfsChange::map_paths` 一次覆盖全部路径（含 `node` 载荷内的路径）。
- **禁止轮询、禁止私有刷新通道**。`created` / `updated` / `deleted` 这类
  粗粒度变更由消费者防抖重拉收敛；`appended` 必须就地增量应用。
  消费端需自行处理「增量与刷新响应竞争」的情形（见
  [vdfs-session-messages.md](../../symbio/src/plugins/session/docs/vdfs-session-messages.md) §S18）。

## 10. 扩展指引

### 10.1 新增一类资源（= 新增一个子目录）

1. 为模块实现 `VdfsProvider` —— **没有必填方法**：
   - 自描述按需：`label`（缺省由使用方以目录名代替）/ `description` / `order`
     / `root_access` / `root_status` / `root_new_types` / `root_hidden`；
   - 数据操作按需实现 `list` / `stat` / `read` / `write` / `delete` / `mkdir`
     / `move_item` / `action` / `watch` / `unwatch`；未实现者保持默认
     （`NotImplemented`）。
   - **不需要、也不应该提供自己的位置**（§2.4）——`impl VdfsProvider for X {}`
     即可编译。
2. 在插件 `traverse` 的 `TRAVERSE_AVAILABLE_TOOLS` 分支中
   `register_vdfs_provider(目录名, provider)`；目录名用插件名（`PLUGIN_*`）。
3. 校验写在 `write` 里；呈现需求放在节点的 `ext` + `schema`。

**零改动面**：访问层、门面（`UnifiedFs`）、物理层、前端（导航 / 列表 / 详情）、
容器（`composite`）都无需改动——容器逐子插件收集，新子目录自动出现在 `<根>`
之下，LLM 侧 `<根>/<插件名>/…` 自动可寻址。
仅当需要一类全新的详情渲染器时才在前端登记一个 ext → 组件映射。

### 10.2 新增一个容器层

需要「容器套容器」（例如某个插件内部再挂若干子资源）时，**不要**新造私有协议、
也不要改访问层：

1. 子容器实现 `VdfsProvider`（可以直接就是另一个 `CompositeVdfs`——composite
   嵌套 composite 时它同样只是普通 provider，没有任何根的概念）；
2. 子容器在 `traverse`中把该视图
   `register_vdfs_provider(自己的插件名, 视图)`（成为父容器下的一个子目录），
   或者由装配决定把它登记进 `register_vdfs_root` 槽位。

两种挂法都由既有机制支持；**composite 的代码对二者完全无感**——这正是
「它没有根级别概念」的意义。

### 10.3 换宿主

重写三个文件即可：

- `symbio_core/vdfs/host.rs`（桥：装上下文、翻错误）；
- `plugins/vdfs/host.rs` + `fs.rs`（访问层与门面：拆信封、地址分流、树遍历、
  事件投递——`UnifiedFs` 只依赖纯接口，可原样带走）；
- 容器的注册接线（子插件收集方式随宿主插件机制而变）。

`symbio_core/vdfs_provider.rs`（纯接口）与 `plugins/vdfs/protocol.rs`（线路信封）
**原样复用**——前者零宿主依赖，后者只依赖前者。

## 11. 与既有机制的关系

- **本地文件**：磁盘文件系统是 `UnifiedFs` 的**物理层**（`plugins/vdfs/physical.rs`
  的 `PhysicalFs`）——原 `local` 插件的 VDFS provider 与其 `SecurityPolicy`
  路径守卫已迁入 vdfs 插件（`FsPolicy`：读写白名单、解析符号链接后复验、
  拒绝写 / 删符号链接、速率限制、读上限、列目录上限）。裸地址（无 `<根>`
  前缀）**直接**落在物理层，「相对路径从工作目录开始」的既有规则原样保留；
  行号分页、`ignore` 过滤、精确字符串替换（保持换行符风格）、文件名 Glob
  均由访问层组合操作对齐。
- **`CapabilityVisitor`**：由「工具 + 模型服务 + 系统提示词」扩展为
  「+ VDFS 注册项 + `<根>` 服务者槽位」。与**选项**（`OptionVisitor`）、
  **可配置**（`ConfigurableVisitor`，§13.1）一样，都搭同一次 traverse 广播的
  便车：三条通道平行，各有自己的 ctx 键与降级行为（收集器缺席 = 当作没人声明），
  不新造广播、也不互相依赖。
- **容器**：容器是「目录 + 拓扑」的自然落点——它实现 `VdfsProvider` 并被装配为
  `<根>` 的服务者。容器**不必**认识任何具体资源：它只逐子插件收集，
  子目录内部层级由各 provider 的 `list` 表达。
- **资源存储（`providers/vdfs_service`）**：属于**宿主实现层**（与
  `providers/embedding` 同层），**不在** §2.1 / §2.2 的纯接口层里——
  `symbio_core/vdfs_provider.rs` 依旧只有 trait 与域类型，零 `use crate::`。
  三个实现（`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs`）本身就是**完整的
  `impl VdfsProvider`**，插件直接组合具体类型，**不走 `create_object` 工厂**
  （不存在第二种实现，套 `dyn` 只是把一次构造换成一次字符串查表）。
  它与 VDFS 广播机制的**唯一接触点**是 `symbio_core::vdfs::host` 的
  `notify_change` / `watch_changes` / `unwatch_changes`；此外它只是一份条目存储
  （寻址 + 原子写 + zip 整包），不认识任何资源语义（§13.4）。
- **详情定义**：`DetailDefinition` 从 VDFS 协议中**移出**，降级为 symbio 的
  `schema` 方言，从而不污染开放接口。

## 12. 一致性要求

- 任何新的资源访问功能 **不得** 绕开 `vdfs/*` 新造私有协议。
- `symbio_core/vdfs_provider.rs` **不得**引入 `crate::` 依赖；宿主专有类型一律经
  `VdfsContext` 注入。
- 线路类型（请求 / 响应信封、`VDFS_OPS`）**不得**上浮到 core；它们只属于
  `plugins/vdfs/protocol.rs`。
- provider **不得**提供或假设自己的位置（trait 上无 `mount()` / `category()` /
  root 概念）；只接收自身子树内的相对路径。
- 子目录名**只能**由注册方在 `register_vdfs_provider(dir, …)` 处给出（约定 =
  插件名）；**不得**出现第二处命名来源。
- **地址规则只归门面**：`<根>` 前缀判别与两半分流**只允许**出现在
  `UnifiedFs`；访问层、工具、provider、容器都**不得**重复实现地址判别或
  前缀拼接。
- **拓扑只归容器**：访问层与门面**不得**持有子目录表、不得解析虚拟路径首段；
  「目录名 → 子树」的映射只允许出现在容器的组合视图与注册接线里。
- **composite 不得出现根级别概念**：它的代码里不得有「自己是根」「只能有一个」
  之类的假设；能否服务 `<根>` 只由装配（`register_vdfs_root` 槽位）决定。
- 运行时状态（workdir 之类）**不得**进入 provider 的路径语义，只能经调用级参数
  （§6.5）透传。
- `<根>` 服务者**只能**经 `register_vdfs_root` 槽位装配；**不得**出现第二个
  登记点。
- 消费者 **不得**按 `kind` 判定能力；只能依据 `access`。
- 前端 **不得**硬编码资源类型清单、标签、路径模板、能力开关；只允许登记
  ext → 渲染器、子目录名 → 图标这类纯 UI 映射。
- 接口变更先改 `vdfs_provider.rs`（trait 是 centerpiece），线路变更先改
  `plugins/vdfs/protocol.rs`，再改访问层与前端两侧契约。

## 13. 范例（实例，非机制组成部分）

### 13.1 设置（setting）——自有分区 + 插件配置清单

- 注册名 = 插件名 `PLUGIN_SETTING`（= `<根>/setting` 子目录）；`root_access = l`
  （清单固定，每一项是叶子文档，不可 `t`）。provider 自身不含位置概念。
- 本插件是**无状态 provider**：`SETTING_SECTIONS` 只登记 `appearance` / `about`
  两个**前端状态自持**的分区（主题、版本信息等数据不在后端），`route` 恒
  `NotFound`。
- 分区节点：`access = r`、`ext` = 分区 id（前端据此直接渲染专属面板，§7）。
  `read` / `write` 对它们恒 `Forbidden`——数据在前端 store。
- 清单 = **各插件自己交出来的配置条目** + 自有分区。顺序上配置在前、`appearance` /
  `about` 垫后：前者是用户在设置页里真正要动手的东西，后者是应用自身的展示项。
  条目的 `kind = setting`、`name` = 插件目录名（前端图标键 `setting:<目录名>`）、
  `path` = 该插件配置文档的**真实地址**（`<插件目录>/PLUGIN.yml`）。所以设置页只是
  「指路」：点开读写的还是拥有者那份文件，本插件不代理读写、也不复制配置。
- 插件配置**不再由本插件代存**：各插件的配置归各插件自己的目录（§3.4），如
  `<根>/session/PLUGIN.yml`、`<根>/local/PLUGIN.yml`。原 `setting/config/get` /
  `setting/config/set` 与 `SETTING_SECTIONS` 里的 4 个插件分区（各自的 `prefix`
  与代理转发）已随之退场——那正是「同一份配置有两个地址」的根源。

**可配置收集通道**（第三条收集通道，与能力 / 选项并列，见 §10.1）：插件在
`traverse` 里调 `announce_configurable(&ctx, &self.config_file)`，声明「我有一份配置
文档」。容器在 `children_of` 的那次广播里用一个**共享**收集器收下——不是像 VDFS
provider 那样每个子插件一个：声明自带目录名，不存在归属歧义。收集结果**写回请求
ctx**，同一次请求里稍后被委派的 provider（即本插件）据此读到清单，因此**无需反查
插件目录、无需硬编码插件表、也不需要协议上的新字段**。条目由
`entry_of(&ConfigFile)` 生成，标题 / `ext` / `schema` 都取自 `ConfigFile::node()`；
**图标不进协议**（前端 `kind:<目录名>` 的纯 UI 映射）。通道缺席时本插件照常只列
自有分区。

### 13.2 组合容器（composite）——包含子目录列表的 provider

- `composite` 实现 `VdfsProvider`（`CompositeVdfs`），`label = "系统"`、
  `order = 0`、`root_access = lt`。**它没有任何根级别概念**：只是一个恰好包含
  若干子目录的 provider（§10.2），当前服务 `<根>` 纯属装配安排。
- 九个操作都是同一件事：现场调用 `children_of(ctx)` 取子目录清单，解析首段后
  委派（`list` / `stat` / `read` / `write` / `delete` / `mkdir` / `move` /
  `action` / `watch` / `unwatch`）。
  **不缓存**——子插件集合由配置与生命周期决定，每次现取才与容器一致。
- `children_of` 逐子插件广播 `TRAVERSE_AVAILABLE_TOOLS`（每个子插件独立收集器），
  汇总为 `(目录名, provider)` 清单（按 `order` 升序）。
- 守卫：自身目录与子目录根不可读 / 写 / 删 / 移、`mkdir` 已存在报错、
  跨子目录移动被拒、子节点路径回填树内全路径、事件相对路径补全（§5）。
- 隐藏属性：合成子目录节点时把子 provider 的 `root_hidden()` 回填进
  `VdfsNode::hidden`，并据此过滤掉不该出现在清单里的子目录（§3.2）；委派回来的
  `list` 结果同样过滤——隐藏是**机制级**属性，不因节点来自哪个 provider 而异。
  `stat` 仍如实报告该属性（隐藏只影响列表，不影响可达性）。当前标为隐藏的是内容
  仅一份配置文档的目录：`web` / `local` / `gateway`（各自的配置地址
  `<根>/<插件>/PLUGIN.yml` 照常可寻址，也照常出现在设置页清单里）。
- 在 `traverse` 中把该视图登记进 `register_vdfs_root` 槽位（§6.2）。

**子插件从哪来：插件目录**（见 `symbio_core::plugin_dir`）。容器是**通用**容器
（可以嵌套另一个容器），子项因此不来自父插件塞进来的配置表，而来自**扫描插件根**：

- 布局：`<homedir>/<插件>/PLUGIN.yml`（配置）+ 该插件自己的数据 / 资源，
  同处一个目录，因此整个目录可直接拷贝移植。系统级插件（`home` / `composite`）的
  目录是**系统根本身**，配置在 `<homedir>/PLUGIN.yml`。
- 加载判据：目录下的 `PLUGIN.yml` 可解析、且 `plugin_provider` 指向已注册的工厂
  （`has_creator`）。不合格的目录跳过并点名。
- 构造者把**插件自身目录**经 ctx 键 `PLUGIN_DIR` 告知被构造的插件；插件据此自己
  读写配置（§3.4），容器不碰它的配置。
- 「必需插件」清单是**构造者的策略**，经 ctx 键 `REQUIRED_PLUGINS` 传入——容器
  不内置任何清单。`home` 声明 `SYSTEM_PLUGINS` 并随构造传入。
- 容器扫描的是**自己的目录**（`PluginDir::as_plugins_root`，= 系统根）：系统级插件
  `home` 的目录就是系统根本身、不与业务插件并列，因此扫描不会构造出第二个 `home`
  （否则自举成环）。

### 13.3 VDFS 插件（vdfs）——访问层与统一文件系统

- 路由：`vdfs/<op>` → `resolve_fs`（构造本次调用的 `UnifiedFs`）→
  `dispatch_with` → 拆信封回包。
- **地址规则（在 `fs.rs`，全项目唯一一份）**：
  1. **虚拟地址**（`<根>` 独占首段）：剥前缀成树内相对路径交给虚拟层根
     （容器登记的组合视图）；`<根>` 本体 = 树内 `""`。
  2. **物理地址**（其余一切）：工作目录相对地址或绝对路径，原样交给
     `PhysicalFs`；workdir 经 `call_params` 透传，缺失即 `Internal`（接线错误）。
  3. **跨半移动**：在分流阶段就拒绝，不触达任何一层。
- **LLM 工具链路（`provider.rs` + `tools/`）**：vdfs 插件封装一个工具链路 provider
  `ToolVdfs`——它**持有 `CapabilityVisitor`**；`traverse` 广播中由 `plugin.rs`
  构造（`ToolVdfs::new(visitor)`）并把工具注册进同一个 visitor。每个工具
  （`tools/` 下一工具一文件，均为框架原生 `Capability`）构造时持有这同一个
  `Arc<ToolVdfs>`，`execute` 内只是「**对文件系统的操作改为对它的操作** + LLM 封装」
  （行号分页 / ignore 过滤 / 成功 message），不认识能力管理器、不走协议信封。
  工具：`vdfs_list` / `vdfs_tree` / `vdfs_stat` / `vdfs_read` /
  `vdfs_edit` / `vdfs_search` / `vdfs_write` / `vdfs_delete` / `vdfs_mkdir` /
  `vdfs_move`，共十个。工具描述中的地址口径：`<根>` = 系统资源（其子目录清单
  由 `vdfs_list('<根>')` 发现），裸地址 = 会话工作目录。
- **`ToolVdfs` 只做两件事**：取根（`root_of`）→ 交给 `UnifiedFs`；透传调用级
  参数（`call_params`）。地址翻译、两半分流、路径回填、根守卫、跨半移动拒绝
  全部在门面一处——历史上「裸地址补 `local/` 前缀 → 再拆挂载名 → 按名取
  provider」的三步翻译已随物理层并入而消失。
- **组合操作只写一次（`host::edit_via` / `host::search_via`）**：`VdfsProvider`
  trait 只含**原子操作**——原生文件系统没有 edit / search 对应的原子调用，
  读改写、遍历过滤属于组合逻辑，放 trait 里会逼每个实现方重复实现。二者
  落在访问层各一份：编辑 = `read` → 精确替换 → `write`；搜索 = 递归 `list` +
  Glob 过滤（安全规则经各层的 `read` / `write` / `list` 自持生效）。前端
  协议入口（`vdfs/edit` / `vdfs/search` handler）与 LLM 工具链路（`ToolVdfs`）
  共用同一份组合实现；`VdfsEditResponse` / `VdfsSearchResult` 因此归
  `plugins/vdfs/protocol.rs`（访问层形状），不属于 core。
- **物理层（`physical.rs`）**：`PhysicalFs` + `FsPolicy`——原 `local` 插件的
  文件 provider 与安全策略整体迁入：读写路径白名单、解析符号链接后复验
  （防链接逃逸）、拒绝写入 / 删除符号链接、速率限制、读上限 10MB、
  列目录上限 2000 项。`list` 目录在前、各自按名升序；文件 `rw`、目录 `lwt`；
  图片扩展名映射为 `binary = true` + MIME，`read` 返回 base64（多模态）。
- 前端链路与工具链路构造**同一个 `UnifiedFs`**（虚拟层根同源、物理层同一块磁盘），
  消费**同一批注册的 provider**，不存在第二套实现。

### 13.4 会话 / 模型 / 技能 / MCP / 智能体——各插件直连 `VdfsProvider`

- **每个资源插件自己就是 provider**：`session` / `model` / `skill` / `mcp` /
  `agent` / `setting` 各自有一份 `impl VdfsProvider`，注册名 = 插件名。
  中间不再有 trait 与适配器——**（历史）**曾有的 `EntityProvider` +
  `EntityVdfsAdapter` 两层，以及其下的 `providers/storage_service`
  （`StorageService` / `EntityStore`）与 `symbio_core::entities` 那组存储原语，
  已先后全部删除（理由见 [DECISIONS](../DECISIONS.md) ADR-010「收敛终局」与
  ADR-011）。
- **跨插件共用的是 `providers/vdfs_service` 的三个 `VdfsProvider` 集中实现**
  ——**三种拓扑、一份磁盘布局**（`<homedir>/<category>/<id>/<manifest>`，
  因此换拓扑不动数据、换类别不碰协议）：

  | 实现 | 一个条目 = | 条目内部 | 消费者 |
  |---|---|---|---|
  | `SingleFileVdfs`（`single_file.rs`） | 一份主文件 | **不外露**（叶子） | `model` |
  | `DirVdfs`（`dir.rs`） | 一个目录 | 可下钻浏览，主文件承载条目内容 | `skill` / `mcp` |
  | `MemoryVdfs`（`memory.rs`） | 进程内一条记录 | 无（不落盘） | `model` 的 VDFS 清单镜像 |

  三者共用 `entry.rs` 的条目寻址与落盘原语（`category_dir` / `safe_segment` /
  `entry_dir` / `split_rel` / `id_of` / `pack_name_of` / `Entry` + 读写删）；
  zip / base64 与导出载荷 `VdfsPack { id, filename, b64 }` 在 `pack.rs`。
  **差异（呈现、写前校验、写后内存同步）仍在各插件自己的 `impl` 里**，由调用点
  以普通 Rust 参数传入——没有新 trait、没有适配器、没有注册表。用哪一型是挂载点
  在构造时**声明**的（类别段名 = 插件名 = `PLUGIN_*`，主文件名 = 插件内部
  `const MANIFEST`），不是机制去目录里看出来的。
- **清单的真相源不止磁盘一种**：`model` 的列表来自内存（`MemoryVdfs` 镜像——启动时
  从磁盘灌入、写盘成功后回灌，镜像与运行时注册表同一处更新）。内存型与磁盘两型的
  操作语义同构、走同一条变更广播频道，消费者分不清也不必分清条目住在哪儿。
- **「manifest 补齐 id」由各插件自己实现**：编辑链路只回纯字段值（id 由路径承载），
  `model` / `mcp` 各有一份 `with_id`——那是该资源的写入语义，不再是跨插件共享原语。
- **目录自管的类型自己落盘**：agent bundle 走 `BundleStore`（工作区级 + 全局级
  双层），直接用 `import` / `export` / `delete`，**不经 vdfs_service**；
  其目录内部（提示词 / 技能 / MCP）以 `<bundle id>/<子类别标签>/<相对路径>`
  寻址，子类别用**人读的标签**（`提示词` / `技能` / `MCP`）而非 kind 作路径段
  ——与会话内部的「子会话 / 工作目录」同一口径。
- **变更广播按类型全局持有**：`vdfs::host::notify_change` / `watch_changes` /
  `unwatch_changes`。落盘的写 / 删（`vdfs_service` 三实现内部）与目录自管型 provider
  都调 `notify_change`，使订阅方无需轮询；按 `kind` 而非 provider 实例持有，是因为
  同一 provider 会被多次构造（每次 `traverse` 一份），共享同一广播才能让订阅与投递
  天然配对。**生命周期与运行时状态变化共用这一条 `kind = "vdfs"` 频道**——曾有第二条
  `kind = "entity"` 频道（状态角标 / 清单增删各报一次），已整体删除：状态变化同样是
  该节点的一次 `updated`，前端收到后重读 `vdfs/stat`（§9 的不带载荷那一档）。
- 详见 [vdfs-frontend.md](vdfs-frontend.md) §7 的 S4 / S6 / S7 与 S17 记录。
