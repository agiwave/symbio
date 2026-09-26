# VDFS —— 虚拟动态文件系统规范

状态：现行规范（纯接口 + 统一文件系统 + 容器拓扑）
范围：Symbio 全部「资源与数据访问」能力（前端 + 大语言模型）
关联：
`symbio/src/symbio_core/vdfs/`（**纯接口，权威定义：`VdfsProvider` trait
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
                      │ register_vdfs_provider(插件名, provider)   ← LLM 链路（CapabilityVisitor）
      ┌───────────────┬─────────────┼─────────────┬───────────────┐
 plugin_manager provider  session provider  model provider   …（各模块自持）
```
> 上图只画了 **LLM 链路**的注册关系。系统链路（前端 / 子智能体挂载点穿越）**不经**
> 此广播：拿到父插件（`Arc<dyn Plugin>`）后直接调 `get_vfs_provider()` 取根，详见 §6.4。

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
symbio_core/vdfs/*.rs          ← centerpiece：VdfsProvider trait
                                  + 域类型（VdfsNode / VdfsContent / VdfsAccess /
                                    VdfsWriteResponse / VdfsError / VdfsChange）
                                  + VdfsContext + 路径工具 + 回填
symbio_core/vdfs/host.rs       ← symbio 桥：上下文注入 + 错误翻译
plugins/vdfs/protocol.rs       ← 线路信封：vdfs/* 请求响应 + 协议路径常量
                                  + VDFS_OPS（变更事件与 provider 侧 VdfsChange 同形）
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

各层的文件与职责见 §2.1 清单。**只允许向下依赖**：纯接口层仅依赖
`std` / `serde` / `serde_json` / `async_trait`（零 `use crate::…`）；宿主桥 / 访问层 /
拓扑用宿主自有类型；线路信封 / 地址分流 / 物理层依赖其下两层。

硬约束：

- **纯接口层不得出现 `use crate::…`**。`vdfs/` 可原样抽出为独立
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

实现方（如 `plugin_manager` 插件）是**插件**而非宿主本身，它需要两件宿主能力：
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
  （系统链路经 `get_vfs_provider()` 取 provider 时**不带目录名**，目录名由 composite
  的实例表挂载名提供——二者都不让 provider 自己决定位置，论点一致。）
- provider 的方法只接收**自身子树内的相对路径**（`""` = 自身目录）；
  全路径 `<子目录>/<rel…>` 由使用方（容器）拼接与回填；
- 子目录节点由容器合成（`dir_node`：携带子 provider 的自述与访问位），
  provider 不参与；
- 变更事件（`VdfsChange`）同样不含子目录名——provider 只报子树内相对路径，
  容器补成树内全路径、门面再补成对外展示地址（§9）。

好处：provider 实现者不必先想一个名字、也不必担心重名；同一个实现可被放在
任何位置；换使用方策略（改名字、改顺序）不需要触碰任何 provider。

### 2.5 拓扑只归容器

「谁在哪个子目录」只由**组合容器**知道。容器把聚合出的视图经**两条通道**暴露，
二者指向同一个 `CompositeVfs` 实例、但服务于不同链路：

- **系统 / 前端链路**（取子插件视图、穿过挂载点委托）：容器实现 core trait
  `Plugin::get_vfs_provider`（默认 `None`）返回 `self.vdfs`。这是系统链路的**唯一**
  查询接口——**不经** `CapabilityVisitor`、不驱动任何 `traverse` 广播。
- **LLM 链路**（LLM 读写挂接）：容器在 `traverse(TRAVERSE_AVAILABLE_TOOLS)` 中把
  视图登记进 `CapabilityVisitor` 的**单槽位** `register_vdfs_root(...)`，供 `vdfs_*`
  工具经 `get_vdfs_root` 取根。

拓扑角色划分：

- **容器**（`composite`）＝ 唯一持有拓扑：系统链路经 `child.get_vfs_provider()`
  逐子插件查询聚合出「包含子目录列表的 provider」，LLM 链路经 `register_vdfs_root` 登记；
- **门面**（`UnifiedFs`）＝ 只认地址前缀：`<根>` → 虚拟层（容器视图），其余 → 物理层；
- **访问层**（`vdfs` 插件）＝ 拆信封 + 取 `UnifiedFs` + 转发；
- **provider**（各插件）＝ 只认自身子树内相对路径。

于是拓扑知识的落点**有且只有一处**（容器），且容器不依赖访问层、访问层与门面
不依赖任何具体资源——四者可以独立替换（§10.3）。`CapabilityVisitor` 与
`Plugin::get_vfs_provider` **不是同一回事**：前者是 LLM 能力挂接通道，后者是
系统链路的查询接口，二者不可混用（详见 §6）。

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

### 3.2 节点与条目

`VdfsNode` 是文件与目录的统一表达——**一份自述，不含地址**：

| 字段 | 语义 |
|---|---|
| `name` | 父内唯一标识（路径段） |
| `title` / `description` | 展示标题 / 语义说明（缺省 `title` = `name`） |
| `kind` | **场景标签**（自由取值；构造器缺省给 `dir` / `file`，场景可覆盖）；机制不据此判定 |
| `status` | `active` / `working` / `disabled` / `failed` / `unknown`；缺省 `active`，**空串 = 显式声明「无运行态」**（`VDFS_STATUS_NONE`，列表不画状态点）。词表权威处是 `symbio_core::vdfs` 的 `VDFS_STATUS_*` <!-- vocab:VDFS_STATUS_ --> |
| `access` | 访问位（§4），**机制唯一的能力依据** |
| `ext` | 呈现扩展名——**前端据此选择详情页面**（§7） |
| `size` / `updated_at` / `children` / `binary` | 元数据 |
| `new_type` | 该目录可新建的**那一种**东西（可选，缺省 = 不可新建；§7 与 vdfs-frontend.md §5） |
| `schema` | 不透明呈现描述（宿主方言，VDFS 透传） |
| `attributes` | 场景扩展字段（flatten 到顶层） |

**目录与文件不做类型区分**：`access` 含 `l` 即可列（目录），含 `r` 即可读
（文件）。`is_dir()` 的实现就是 `access.list`。

**`hidden` 与文件系统的隐藏属性同义**，是**机制级的节点属性**：任何节点都可以带，
与它是不是目录、属于哪个场景无关——provider 的根**本来就是一个目录节点**，所以
「它在父目录的清单里显示还是隐藏」与文件 / 目录是同一件事。语义边界只有两条：

- **列表里不出现**：父目录的 `list` 结果不含它（`children` 计数同理）；
- **可达性不受影响**：按路径 `stat` / `read` / `write` / 子树操作一概照常。

因此「隐藏」既不是权限（那是 §4 的访问位），也不代表节点不在系统里——它依然存在，
只是清单里不列。**过滤发生在产出列表的一方**：容器对自己合成的子目录清单、以及任何
子 provider 交回来的 `list` 结果都做同一条过滤，所以标了 `hidden` 的子节点不因来自
哪个 provider 而异。缺省不隐藏（序列化时省略）。

典型用法：内容只有一份配置文档、没有用户资源可浏览的目录
（如 `<根>/web` / `<根>/local` / `<根>/gateway`）不必出现在导航列表里，
但它们仍按 `<根>/<插件>/PLUGIN.yml` 完全可寻址。

`kind` 与目录性**无关**：`dir` / `file` 只是构造器（`VdfsNode::dir` / `file`）给出的
**缺省场景标签**，场景想表达别的语义（插件名、条目类别…）就直接覆盖它。因此
`kind` 只有一个词表——「场景标签」，不存在「基础类型 + 场景类型」两套口径；目录性
永远只由 `l` 位表达。

**保留字段名**：`attributes` 会 flatten 到条目顶层，因此机制字段名是保留字，场景
扩展字段**不得**与之同名（否则扁平化后互相覆盖）：

```text
path  name  title  description  kind  status  access  ext
size  updated_at  children  binary  hidden  schema  new_type  attributes
```

新增机制字段时须同步本节；场景侧的命名建议带上自己的前缀（如 `config_type`、
`meta_tags`），但机制不做强制。

#### 条目：地址属于列表，不属于节点

`path` **不在** `VdfsNode` 上。地址是**某一份列表**给这个节点的定位，不是节点
自己的属性——同一个节点可以在不同列表里以不同地址出现。实证：设置页的一项指向
插件自己那份配置文档 `<目录名>/PLUGIN.yml`（落在**另一个挂载点**里），而同一份
文档在自己的目录里就叫 `PLUGIN.yml`。

于是清单里的每一项是 `VdfsItem`（`path` + `VdfsNode`，`#[serde(flatten)]`，
**线格式与「带 `path` 的节点」逐字节相同**）：

- 常规情况地址**由分发层按 `<父地址>/<name>` 回填**，provider 不填；
- 只有「地址不是那个形状」时（设置页条目）才由**拥有者**自己填，分发层原样透出。

读写 `stat` / `read` / `write` 回来的都是**纯节点**——地址在请求里已经有了。

### 3.3 内容

`VdfsContent` 承载文本或二进制（互斥）：`text` 或 `b64`，由 `binary` 显式标注，
不做猜测；`size` / `mime` / `etag`（乐观并发令牌，可选实现）。**不带地址**——
内容总是「请求的那个节点」的内容，地址在请求里已经有了。

#### 写意图 `create` 与两种目标形态

`vdfs/write` 的 `path` 可以指向**两种东西**，实现方都必须考虑：

- **具名节点**（`<名字>` / `<父>/<名字>`）：常规的「写这个节点」；
- **目录自身**（`""` = 自身根，或任何指向目录的地址）：使用方**没有给名字**
  ——「新建一个，叫什么由你定」。**这是「新建」在机制上的形态**：使用方只说
  建在哪个目录，不说叫什么（名字是 provider 的私有知识）。因此**写目录自身不是
  错误**：provider 生成一个名字（id 归 provider）、落盘、并**在
  `VdfsWriteResponse.name` 里给出那个名字**——这是唯一 provider 才知道的一件事。
  地址由使用方拿自己的请求目录 + 这个名字拼（写文件不需要 API 回传完整路径）。
  不支持（该目录没有可新建的类型）则照常报错。具名写回 `name: null`。

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

**导入 / 导出是一对逆向的节点动作**：两者都不新增操作，都走 `vdfs/action`，
都只在详情页上占一条动作——与「删除」「测试连接」同级：

- `export`：把条目打包成 zip，随 `VdfsActionResult.data` 回传
  （载荷 `{id, filename, b64}`，与 `VdfsContent.b64` 同构）；
- `import`：收一个 `{filename, b64}`（入向 `VdfsUnpack`，与出向 `VdfsPack` 同形），
  由 provider 解释为「用这个包建出 / 覆盖本目录下的一份资源」。

**导入不是「新建类型」的一种**，因此 `VdfsNewType` 里没有它的位置
（见 [DECISIONS.md](../DECISIONS.md) ADR-029）。动作声明 `pack` 表明「载荷是一个
本地文件」，使用方据此先取文件再执行；provider 不支持时返回 `NotImplemented`，
使用方据此不给出入口（详见 [vdfs-frontend.md](vdfs-frontend.md) §5）。

> 目录型资源的**存储层原语**仍接受二进制写入（`vdfs/write { b64 }` = 解包），
> 但那是对内的原语，不是对外的入口形态——对外只有 `import` 动作一条路。

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
- **落盘就是写自己的文件**：`PluginConfigFile::apply` 的一条链是
  **校验 → 落内存 → 落自己的文件 → 广播**。没有 `save_config` 路由、没有切片推送、
  没有父插件参与——配置回到拥有者手上，父插件不认识任何子插件的配置。
- **身份字段是保留键**：`plugin_provider`（工厂 id）/ `plugin_name`（实例名）与配置
  字段同处一个文件，但**不参与配置反序列化**（`PluginDir` 读写时自动剥离 / 补回）。
  装配方据此判定「这个目录是不是一个可加载的插件」：文件存在、可解析、且
  `plugin_provider` 指向一个已注册的工厂（见
  [composite/README.md](../../symbio/src/plugins/composite/README.md)）。
- **副作用留在插件**：写完配置之后还要做什么（如网关重建监听）在插件自己的
  `write` 里做——`PluginConfigFile::apply` 返回后再执行，机制不引入回调抽象。
- **实现只写一次**：`symbio_core::plugin::dir` 的 `PluginDir`（目录 + 配置读写，
  含遗留字段清理）与 `PluginConfigFile`（节点形状 + 定义校验 + 落盘）——
  **值 + 一组函数**，不是 trait：没有注册表、没有回调。
- **凭据在配置里**：配置文件与其它节点一样受访问位约束（`r`），即**可读**。
  对外暴露面（如网关的只读白名单）需自行拒绝落在 `<插件目录>/PLUGIN.yml` 上的读取。
- **设置页只是「指路」**：设置插件的清单里会出现这些配置文档（见
  [plugin_manager/README.md](../../symbio/src/plugins/plugin_manager/README.md)），但条目
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
core 不暴露**）；操作闭集见 [CURRENT.md](../CURRENT.md) §3.2，信封的线上形状以
`protocol.rs` 为准。语义要点：

- `vdfs/root` **不给地址**：回包 `path` 即根地址（消费方当运行期数据持有，§3.1）。
- `vdfs/tree` 是访问层按 `t` 位递归 `list` 的**机制级**通用能力；`vdfs/edit`
  （`read` → 精确替换 → `write`）与 `vdfs/search`（递归 `list` + Glob）是访问层的
  **组合操作**，前端协议入口与 LLM 工具链路共用同一份实现。
- `vdfs/action` 执行 provider 自持的**节点动作**，未实现返回 `NotImplemented`
  （使用方据此隐藏入口）；`vdfs/watch` / `vdfs/unwatch` 严格配对（§9）。
- **每个操作都是对「统一文件系统」的一次方法调用**，访问层不含任何资源语义；
  **不存在** `vdfs/providers` 这类挂载清单端点——`<根>` 的 `list` 就是子目录清单。

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

- 虚拟层内：自身目录与子目录根不可**读 / 删**，也不可 `mkdir`；
  ⚠️ **写不在其中**——写子目录根 = 写在**挂载点目录自身**上（§3.3「写目录自身」），
  容器一律转发给子 provider 判定，不做类型特判；
- 子节点 / 内容 / 写入响应的 `path` 回填为全路径（树内 → 展示口径）；
- 变更事件的相对路径补全为展示地址。

**移动不在协议里**：`VdfsRequest` 只收**操作载荷**，地址一律走 `path` 参数——载荷里
没有第二个地址字段。跨虚拟挂载树的「移动」本质是 `copy + delete`，不是核心原语，
故整条链（`vdfs/move` / `vdfs_move` 工具 / 前端重命名入口）都不提供；要用移动由
**外层组合**（理由见 `symbio_core::vdfs::request` 的「没有 `Move`」一节）。

错误经 `VdfsError` 表达，桥层翻译为宿主错误（symbio → `PluginError`）。
`VdfsError::Invalid(VdfsValidationError)` 的结构化字段级错误序列化为 JSON 置于
错误文案位，前端可解析后逐字段高亮；解析失败按纯文本展示（向前兼容）。

## 6. 注册与收集

### 6.1 provider 注册（两条链路，两套通道）

> ⚠️ **两套发现通道不可混用**：
> - **LLM 链路**经 `CapabilityVisitor`（`traverse` 广播中注册 / 查询）；
> - **系统 / 前端链路**经 core trait `Plugin::get_vfs_provider`（直接查询，**不**走 `CapabilityVisitor`、不驱动 `traverse`）。

**LLM 链路**——`CapabilityVisitor` 上的方法（均有默认实现）：

```rust
// 目录名 = 使用方选定（约定 = 插件名），provider 自身不含此概念
async fn register_vdfs_provider(&self, dir: &str, provider: Arc<dyn VdfsProvider>);
async fn list_vdfs_providers(&self) -> Vec<(String, Arc<dyn VdfsProvider>)>;  // order 升序
async fn get_vdfs_provider(&self, dir: &str) -> Option<Arc<dyn VdfsProvider>>; // 按目录名查
// 容器自身视图登记进单槽位：
async fn register_vdfs_root(&self, provider: Arc<dyn VdfsProvider>);
async fn get_vdfs_root(&self) -> Option<Arc<dyn VdfsProvider>>;
```

> 消费现状（写清楚，防误判）：当前 `vdfs_*` 工具取根**只消费根单槽**
> （`get_vdfs_root` → `UnifiedFs` 整棵组合视图）；**按名注册的挂载清单**
> （`list_vdfs_providers` / `get_vdfs_provider`）暂无消费方——它是 **LLM
> 可控挂载机制的预留面**（子智能体经 `SubAgentVisitor` 在此加 `agent/<id>/`
> 作用域前缀；将来按作用域 / 白名单裁剪 LLM 可见资源时，消费方接在这组接口上）。
> **不是死代码，不要删除。**

**系统 / 前端链路**——core trait `Plugin`（默认 `None`，资源插件返回 `Some(self)`）：

```rust
// 系统链路取「本插件自身 VDFS 视图」的唯一接口；无 VDFS 的插件保持默认 None
fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>>;
```

LLM 链路：插件在 `traverse` 的 `TRAVERSE_AVAILABLE_TOOLS` 分支里，**在注册工具的同一处**
顺带注册自己的资源，目录名用插件名——`register_vdfs_provider(PLUGIN_ID_MANAGER, self.clone())`。
系统链路：资源插件在 `impl Plugin for X` 直接 `return Some(self)`，无需任何遍历或
注册——容器 `children_of` 与前端 `resolve_fs` 取视图时直接调 `.get_vfs_provider()`。

### 6.2 根登记（容器 → 系统）

`<根>` 的服务者在**两条链路**上各有一处装配点：

- **LLM 链路**：`CapabilityVisitor` 的**单槽位** `register_vdfs_root` / `get_vdfs_root`
  （见 §6.1）。`composite` 在 `traverse(TRAVERSE_AVAILABLE_TOOLS)` 广播里登记：
  ```rust
  if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
      if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
          visitor.register_vdfs_root(self.vdfs.clone()).await;   // 装配安排（LLM 链路）
      }
  }
  ```
- **系统 / 前端链路**：容器实现 `Plugin::get_vfs_provider` 返回 `self.vdfs`，前端
  `resolve_fs` 直接 `parent.get_vfs_provider().unwrap_or_else(empty_root)` 取根。
  此路径**不**经过 `CapabilityVisitor`、不驱动 `traverse`。

**这不是「根级 provider」概念**：槽位 / 查询接口只是装配点——谁被登记 / 返回，
谁的子树就成为 `<根>` 之下的内容。composite 可被别的目录包含、子插件也可以是另一个
composite；登记本身不改变 composite 的任何行为与代码（见 §2.5）。

### 6.3 组合视图：容器如何聚合子插件

容器**本身就是**「包含子目录列表的 provider」（`CompositeVdfs`，无任何中间结构）：
逐子插件经 `Plugin::get_vfs_provider()` **查询**（系统链路，非广播、不驱动
`traverse`），汇总为 `(目录名, provider)` 清单，目录名 = 实例表挂载名（按 `order`
升序），天然归属该子插件。**不缓存**——子插件集合由配置与生命周期决定，每次现取
才与容器一致。

### 6.4 两条消费链路，分别走各自通道

**系统链路与 LLM 链路是两条独立的发现通道**，不共用同一次遍历广播：

- **LLM 链路**：会话编排方 `collect_capabilities` 广播 `traverse(TRAVERSE_AVAILABLE_TOOLS)`
  → 容器经 `register_vdfs_root` 登记 `<根>` 服务者、各插件经 `register_vdfs_provider`
  登记自身资源 → `tool_ctx` 携带该访问器 → `vdfs_*` 工具经 `visitor.get_vdfs_root()`
  取根，交给 `UnifiedFs`。
- **系统链路（前端 / 子智能体挂载点穿越）**：`vdfs` / `agent` 插件拿到父插件
  （`Arc<dyn Plugin>`）后直接调 `parent.get_vfs_provider()` 取根，不广播、不依赖
  `CAPABILITY_VISITOR`（`resolve_fs` 在 `ctx` 无能力管理器时即走此通道）。
- **子智能体挂载点穿越**：`agent/<id>` 是挂载点，`agent` 插件对 `RelPath::Agent`
  （首层）与 `RelPath::File`（子路径）均经 `sub_agent(id).get_vfs_provider()` 委托给
  子 composite 的 `CompositeVfs`；`PluginMeta::hidden` 等可见性过滤由子 composite
  统一执行（分形、与系统根同构）。
  - **变更帧路径**：子 provider 报的是子树相对地址，每层容器继续续接自己的挂载段
    （`agent` 插件补 `<id>`、父 composite 补 `agent`、`UnifiedFs` 补根名），
    合起来即 `agent/<id>/<子目录>/<相对路径>`。
  - **条目地址不由挂载点填**：`agent/<id>` 是挂载点，它给出的前缀是**本插件空间内**
    的相对段（不含 `agent`），填进条目地址就是一个少了外层挂载段的假地址——而访问层
    见到非空地址即认为「拥有者已填好」、不再按请求地址回填。故委托回来的条目地址
    **原样透出**（空则留空），由访问层按请求地址补全为完整地址。
    见 `agent/host/vdfs.rs::list_at` 与 `composite/vdfs.rs` 的
    `container_leaves_item_addresses_untouched`（同一条契约）。

`Plugin::get_vfs_provider` 默认 `None`；`CapabilityVisitor::get_vdfs_provider`（按目录名
取、供 LLM 工具）与它是**两套不同接口**，互不替代。两条链路因此经过**同一个
`UnifiedFs`**（不变量 5）——不存在第二处地址规则。取不到根（无组合容器）时虚拟层
降级为**空目录**（`EmptyVdfs`）：`<根>` 可列出但无内容，具体路径 `NotFound`；物理层
与它无关，照常可用。于是「系统没有资源」与「资源为空」表现一致，前端与 LLM 都不必特判。

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

后端下发的 `ext`（扩展名）是**前端选择详情渲染器的唯一键**；扩展名缺省由 `name`
推导（`a.md` → `md`），provider 可显式覆盖（设置分区显式声明 `form`）。机制只透传
`ext` 与 `schema`（不透明 JSON，symbio 方言 = `DetailDefinition`），不解释它们。
后端下发的 `ext` 与 `schema` 是唯一呈现契约：`form` → 定义驱动表单、`session` →
会话工作区、`md` / `json` / `text` → 文本编辑器、其他 → 通用兜底。

- 前端只持有**纯 UI 映射**（ext → 渲染器、子目录名 → 图标），不硬编码资源类型 /
  标签 / 能力 / 路径模板。
- **「新建」与「选中」进入同一个详情页**：父目录节点的 `new_type`（§3.2）用三个
  字段描述**落成后的节点**——`ext` = 地址后缀（provider 按它剥条目 id）、
  `node_ext` = **渲染器键**（缺省 = `ext`）、`schema` = `form` 渲染器所需的定义。
  使用方据此在还没创建时就能渲染详情页；漏 `node_ext` 落通用兜底，漏 `schema`
  渲染空表单。

前端分层（`schemas/vdfs.ts` → `registry/vdfsTypes.ts` → `registry/vdfsRenderers.ts`）、
页面与路由（`views/VdfsView.vue`，`/vdfs/:dir(.*)*`）以及「新增一类资源前端零页面
开发」的完整契约，见 [vdfs-frontend.md](vdfs-frontend.md)。

## 8. 校验职责

**校验属于 provider，不属于机制**：

- 机制只做与资源语义无关的守卫（§5）。
- provider 在 `write` 内完成必填 / 范围 / 枚举 / 类型 / 跨字段校验，失败返回
  `VdfsError::Invalid(VdfsValidationError { message, fields })`。
- **校验先于副作用**：校验不过时 provider 不得触达落盘或任何下游写入
  （配置文件的 `PluginConfigFile::apply` 即「先 `decode` 校验 → 再落内存 → 最后落自己的
  文件」；校验不过时内存与磁盘都不动）。
- 前端据 `fields[].field` 逐字段提示，与 `schema` 中的字段键对齐。

## 9. 实时性

- provider 在 `watch(path, sink)` 中开始监听，变化时调用 `sink(VdfsChange)`
  上报；`unwatch` 严格配对。
- **provider 报出的 `VdfsChange` 不含子目录名**：`path` 是该 provider 子树
  内的**相对路径**（与 `list` / `stat` 同一坐标系）。位置是使用方的概念（§2.4），
  provider 无从得知。
- **`watch(path, sink)` 的 `path` 也是同一坐标系**（provider 根相对，`""` = 自身根）。
  provider 上报的路径**不相对被订阅的 `path` 收敛**——容器回填时只补**挂载名**
  （首段），不补被订阅的整条路径。两处若都做前缀处理就会重复拼接：

  ```text
  watch("session/abc/message") → 容器拆出 dir="session"、rel="abc/message"
  provider 报 "abc/message/m1" → 容器补成 "session/abc/message/m1"  ✓
  provider 若报 "m1"       → 容器补成 "session/m1"          ✗
  ```

  代价是订阅者会收到兄弟子树的变更（订阅一个会话会看到别的会话的事件），
  由消费者按路径前缀自行过滤——`list` / `stat` 亦然，坐标系因此始终只有一个。
- 消费能力由访问位与 provider 实现共同决定；默认 `watch` 为 no-op（无实时能力
  的 provider 直接成功，消费者退回手动拉取）。
- **订阅登记按 `kind` 全局持有，投递是同步的**（`vdfs::host::VdfsChangeSubscriptions`）：
  每个自管变更源的 provider 持有一份表实例，`watch` 记一条 `(path, sink)`、
  `unwatch` 撤一条，变更点直接 `notify` 遍历投递——**没有中间广播通道、没有转发任务**
  （同步投递**顺序确定**：变更点返回即投递完成，不存在「已落盘但尚未转发」的中间态）。
  表按 `kind` 而非实例持有——同一 provider 每次 `traverse` 都会新构造。
- **重叠订阅按「最具体者优先」恰好一次**：同一条变更若同时落在 `…/abc` 与
  `…/abc/message/m1` 两条订阅之下，只投给路径更长的那条。「一次」指**总线上的一次
  发布**——sink 把变更发进 `kind = "vdfs"` 频道，前端各消费者再按自己的作用域前缀
  过滤，因此收敛为一条不会让任何人漏收，也避免同一变更被发布两次。
- 宿主侧投递分两跳，**各补一次它那层才知道的信息**：
  1. **容器**（`CompositeVdfs::watch`）把 provider 的相对路径**补成树内全路径**
     （`<子目录>/<rel>`）后交给上层 sink；
  2. **门面**（`UnifiedFs`）把树内全路径**补成对外展示地址**
     （`<根>/…` 或物理地址）后**原样**发布到全局事件总线
     （`kind = "vdfs"`，无会话关联、不入回放缓冲）。信封与 provider 侧的
     [`VdfsChange`] 逐字同形——**不存在** `VdfsChangeEvent` 这类影子事件类型。

  前端 `subscribe({ kind: 'vdfs' })` 按 `path` 前缀与**载荷形状**自行分流：
  `data` 含 `delta` 的消息帧**就地追加**（零回读），`data` 为节点视图的运行态帧
  就地落定，无载荷变更防抖重拉。
- **`vdfs_notify_change(kind, path)` 只发无载荷变更**（绝大多数资源信号长这样）。
  带业务载荷的变更由**生产者直接经它已持有的订阅表**投递
  （`VdfsChangeSubscriptions::notify(&VdfsChange::with_data(path, data))`，
  见 `session::transcript::Transcript::emit`）——带载荷是一个**显式动作**，不是
  默认行为；**不存在** `notify_change_with_data` 这类对称门面。
- **变更词汇：信封没有操作枚举**——形状是 `path` + 可选 `data`，
  语义全在 `data` 的字段上：

  | `data` 形状 | 生产者 | 语义 / 消费者动作 |
  |---|---|---|
  | `ChatMessage`（含 `delta`） | 消息域（`Transcript::apply`） | 尾部追加，**零回读** |
  | `ChatMessage`（全量：`content` / 状态 / 身份） | 同上（首帧发图里合并后的全量副本） | 整条替换 / 状态迁移，按字段落地 |
  | `ChatMessage`（`status = removed`） | 同上 | 就地移除——删除是**消息词汇里的状态** |
  | `VdfsNode` | 会话运行态（`emit_session_state`，与 `stat` 同源构造） | 全量节点视图就地落定，零回读 |
  | 缺失 | 全部资源信号（`vdfs_notify_change`） | 「这条路径变了」——回读 / 重拉（幂等）；资源删除回读 `NotFound` 即删除 |

  **`path` 恒为被变更节点自身的地址**：会话是容器，其下是若干并列集合（消息 /
  子会话 / 记忆 / 工作目录……），集合项地址统一为 `<sid>/<集合段>/<项 id>`——
  末段就是节点身份（与 `data.id` 同一事实，以地址为准），因此无需「目录 + 载荷里的
  id」这种二次寻址。无载荷变更的 `path` 同样是节点自身地址。

  **`delta` 是传输形态，不是节点形态**：节点正文的权威形态永远是 `read` 的产物
  （图里只留累积后的 `content`），`delta` 只描述「这一帧到达了哪些字符」。

  **判据：一个变更取值（或载荷字段）必须有生产性生产者，否则它不是词汇的一部分。**
  `data.delta` 的生产者是消息域（`Transcript::apply`）；`data = VdfsNode` 的生产者是
  会话运行态（`Transcript::emit_session_state`）。**信封没有操作枚举**（理由与被
  否决的方案见 [ADR-025](../DECISIONS.md)）——`data` 描述「业务数据变成了什么」才是
  消费端消费的，操作枚举只会让每个消费端多背一次分派。三条机制支撑它：

  - **顺序是节点属性**：`ChatMessage.seq` 是消息在文件夹里的位置，前端按它排序，
    到达顺序与显示顺序无关（两个并行工具的变更混着到、后生成的先到，显示都正确）；
  - **载荷只给热路径**：逐 token 的增量帧全部带 `delta`，创建 / 终态 / 压缩等
    低频帧才回读——取决于它落在热路径还是冷路径上；
  - **删除由既有词汇承担**：删掉的节点「载荷缺失 + 回读 `NotFound`」即删除，
    消息则是 `ChatMessage.status = removed`。

  - `map_paths` 仍是路径翻译的**唯一入口**（见 [ADR-015](../DECISIONS.md)）：
    使用方补挂载前缀时一律调它，不逐字段重建——后者会在新增路径字段时静默漏翻。
  - **禁止轮询、禁止私有刷新通道。** 「禁止轮询」指**无触发的定时拉取**；由变更
    **触发**的回读（含 resync 后的整份重读）不是轮询，而是本机制的正常一半。
    粗粒度变更由消费者防抖重拉收敛——重拉**幂等**，因此无序、可丢、可重放都无害。
    列表**内容**的实时面**属于本通道**：`delta` 覆盖热路径，其余回读。

## 10. 扩展指引

### 10.1 新增一类资源（= 新增一个子目录）

1. 实现 `VdfsProvider`（trait 只有一个方法 `dispatch(ctx, path, req)`）：
   - 按 `VdfsRequest` 变体（`List` / `Stat` / `Read` / `Write` / `Delete` / `Mkdir` /
     `Action` / `Watch` / `Unwatch`）`match`，只实现自己支持的操作，其余返回
     `NotImplemented`（使用方据此隐藏入口）；**match 编译期穷尽**。
     （**没有 `Move`**：地址一律走 `path`，跨子树移动是 `copy + delete`，见 §5。）
   - **自述不在 trait 上**：展示名 / `order` / `icon` / `hidden` / `root_access` 放
     `PluginMeta`；「根下可新建的那一种东西」放**节点自述** `VdfsNode::new_type`
     （根经 `Stat("")` 取，更深层在自己的 `list` 里带，同一条通道；缺省 `None` =
     不可新建。见 [ADR-030](../DECISIONS.md)）。
   - **不提供、也不假设自己的位置**（§2.4）。
2. 同时接好两条发现链路（独立，不可只接其一）：**系统链路** override
   `Plugin::get_vfs_provider` 返回 `Some(self)`（默认 `None`）；**LLM 链路**在
   `traverse(TRAVERSE_AVAILABLE_TOOLS)` 中 `register_vdfs_provider(插件名, provider)`
   （§6.1）。
3. 校验写在 `write` 里；呈现需求放在节点的 `ext` + `schema`。

**零改动面**：访问层、门面（`UnifiedFs`）、物理层、容器（`composite`）、前端
（导航 / 列表 / 详情）都无需改动——新子目录自动出现在 `<根>` 之下、LLM 侧
`<根>/<插件名>/…` 自动可寻址；仅当需要一类全新的详情渲染器时才在前端登记一个
`ext` → 组件映射。工具与操作闭集见 [CURRENT.md](../CURRENT.md) §2 / §3。

### 10.2 新增一个容器层

需要「容器套容器」（例如某个插件内部再挂若干子资源）时，**不要**新造私有协议、
也不要改访问层：

1. 子容器实现 `VdfsProvider`（可以直接就是另一个 `CompositeVdfs`——composite
   嵌套 composite 时它同样只是普通 provider，没有任何根的概念）；
2. 子容器在 `impl Plugin` 中 override `get_vfs_provider` 返回 `Some(self.vdfs.clone())`
   （系统链路发现），并在 `traverse` 中把该视图
   `register_vdfs_provider(自己的插件名, 视图)`（成为父容器下的一个子目录，LLM 链路），
   或者由装配决定把它登记进 `register_vdfs_root` 槽位。

两种挂法都由既有机制支持；**composite 的代码对二者完全无感**——这正是
「它没有根级别概念」的意义。

### 10.3 换宿主

重写三个文件即可：

- `symbio_core/vdfs/host.rs`（桥：装上下文、翻错误）；
- `plugins/vdfs/host.rs` + `fs.rs`（访问层与门面：拆信封、地址分流、树遍历、
  事件投递——`UnifiedFs` 只依赖纯接口，可原样带走）；
- 容器的注册接线（子插件收集方式随宿主插件机制而变）。

`symbio_core/vdfs/`（纯接口）与 `plugins/vdfs/protocol.rs`（线路信封）
**原样复用**——前者零宿主依赖，后者只依赖前者。

## 11. 与既有机制的关系

- **本地文件**：磁盘文件系统是 `UnifiedFs` 的**物理层**（`PhysicalFs` + `FsPolicy`，
  规则见 [vdfs/README.md](../../symbio/src/plugins/vdfs/README.md)）。裸地址（无 `<根>`
  前缀）**直接**落在物理层，相对路径从工作目录开始；行号分页、`ignore` 过滤、精确
  字符串替换（保持换行符风格）、文件名 Glob 均由访问层组合操作对齐。
- **三条收集通道**：`CapabilityVisitor` 与 `OptionVisitor`、`ConfigurableVisitor`
  搭同一次 `traverse` 广播的便车，各有自己的 ctx 键与降级行为（收集器缺席 = 当作
  没人声明），不新造广播、也不互相依赖。
- **容器**：实现 `VdfsProvider` 并被装配为 `<根>` 的服务者，只逐子插件收集；子目录
  内部层级由各 provider 的 `list` 表达（§2.5）。
- **资源存储（`providers/vdfs_service`）**：属**宿主实现层**，**不在** §2.1 / §2.2 的
  纯接口层里；三型 `VdfsProvider` 实现（单文件 / 目录 / 内存）与磁盘布局见
  [CURRENT.md](../CURRENT.md) §4，选型理由见 [DECISIONS.md](../DECISIONS.md)
  ADR-010 / ADR-011。它与广播机制的**唯一接触点**是 `symbio_core::vdfs::host` 的
  `vdfs_notify_change` / `vdfs_watch_changes` / `vdfs_unwatch_changes`。
- **详情定义**：`DetailDefinition` 是 symbio 的 `schema` **方言**，VDFS 只透传、不解释。

## 12. 一致性要求

- 任何新的资源访问功能 **不得** 绕开 `vdfs/*` 新造私有协议。
- `symbio_core/vdfs/` **不得**引入 `crate::` 依赖；线路类型（请求 / 响应信封、
  `VDFS_OPS`）**不得**上浮到 core；宿主专有类型与状态一律经 `VdfsContext` 注入。
- provider **不得**提供或假设自己的位置（trait 上无 `mount()` / `category()` / root
  概念），只接收自身子树内的相对路径。
- 子目录名**只**由使用方给出（LLM 链路在 `register_vdfs_provider(dir, …)` 处，
  系统链路取容器实例表的挂载名），**不得**出现第二处命名来源。
- **地址规则只归门面**：`<根>` 前缀判别与两半分流只允许出现在 `UnifiedFs`。
- **拓扑只归容器**：访问层与门面不得持有子目录表、不得解析虚拟路径首段。
- **composite 不得出现根级别概念**：能否服务 `<根>` 只由装配（`register_vdfs_root`
  槽位）决定。
- 运行时状态（workdir 之类）**不得**进入 provider 的路径语义，只经调用级参数（§6.5）。
- 消费者 **不得**按 `kind` 判定能力，只能依据 `access`。
- 前端 **不得**硬编码资源类型 / 标签 / 路径模板 / 能力开关，只登记 ext → 渲染器、
  子目录名 → 图标这类纯 UI 映射。
- 接口变更先改 `vdfs/provider.rs`（trait 是 centerpiece），线路变更先改
  `plugins/vdfs/protocol.rs`，再改访问层与前端两侧契约。

## 13. 范例（实例，非机制组成部分）

机制与具体子目录无关：下列实例的增删改不影响本规范效力，机制细节各由其模块 README 持有。

| § | 实例 | 它是一份什么 `VdfsProvider` | 机制细节 owner |
|---|---|---|---|
| 13.1 | 设置（plugin_manager） | 只读清单：自有分区 + 各插件配置条目 | [plugin_manager/README.md](../../symbio/src/plugins/plugin_manager/README.md) |
| 13.2 | 组合容器（composite） | `CompositeVdfs`——恰好包含若干子目录的 provider | [composite/README.md](../../symbio/src/plugins/composite/README.md) |
| 13.3 | VDFS 插件（vdfs） | 访问层 + 统一文件系统（`UnifiedFs`） | [vdfs/README.md](../../symbio/src/plugins/vdfs/README.md) |
| 13.4 | 资源插件与资源存储 | session / model / skill / mcp / agent 各自 `impl VdfsProvider`；落盘共用 `providers/vdfs_service` 三型（单文件 / 目录 / 内存） | 各模块 README（[agent](../../symbio/src/plugins/agent/README.md) 等）；[DECISIONS.md](../DECISIONS.md) ADR-010 / ADR-011 |

> 拓扑见 [SYSTEM_MAP.md](../SYSTEM_MAP.md)；实例 × 挂载点的权威清单见 [CURRENT.md](../CURRENT.md)。
