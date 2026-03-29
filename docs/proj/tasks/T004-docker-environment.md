# T004: Docker 执行环境搭建

## 基本信息

| 属性 | 值 |
|------|-----|
| 任务ID | T004 |
| 标题 | Docker 执行环境搭建 |
| 阶段 | Phase 1: 核心能力 (MVP) |
| 优先级 | P0 |
| 预估工时 | 16h |
| 状态 | pending |
| 依赖 | T001 |

## 任务描述

搭建基于 Docker 的代码执行环境，确保代码在隔离、安全的容器中运行，支持生信分析常用工具。

## 验收标准

- [ ] Docker 镜像构建成功
- [ ] 常用生信工具安装完成
- [ ] 代码执行隔离安全
- [ ] 资源限制配置完成
- [ ] 与 Tauri 后端集成成功

## 技术要求

### Docker 镜像设计

```dockerfile
# Dockerfile
FROM ubuntu:22.04

# 安装基础工具
RUN apt-get update && apt-get install -y \
    wget \
    curl \
    git \
    python3 \
    python3-pip \
    r-base \
    && rm -rf /var/lib/apt/lists/*

# 安装 Python 生信库
RUN pip3 install \
    numpy \
    pandas \
    scipy \
    scanpy \
    anndata

# 安装 R 生信包
RUN R -e "install.packages(c('DESeq2', 'edgeR', 'limma'), repos='https://cloud.r-project.org/')"

# 安装生信工具
RUN wget https://ftp.ncbi.nlm.nih.gov/blast/executables/blast+/LATEST/ncbi-blast-*.tar.gz

# 设置工作目录
WORKDIR /workspace

# 设置用户（非 root）
RUN useradd -m analyst
USER analyst
```

### 安全机制

```rust
// 执行配置
struct ExecutionConfig {
    // 资源限制
    cpu_limit: f32,        // CPU 核心数限制
    memory_limit: u64,     // 内存限制 (MB)
    time_limit: u64,       // 时间限制 (秒)
    
    // 安全限制
    network_disabled: bool, // 禁用网络
    read_only_paths: Vec<String>,  // 只读路径
    writable_paths: Vec<String>,   // 可写路径
    
    // 命令过滤
    blocked_commands: Vec<String>,  // 禁止的命令
}
```

### 危险命令拦截

```rust
// 危险命令检测
fn is_dangerous_command(command: &str) -> bool {
    let dangerous_patterns = [
        "rm -rf /",
        "rm -rf ~",
        "mkfs",
        "dd if=",
        "> /dev/",
        "chmod 777",
        "chown root",
    ];
    
    dangerous_patterns.iter().any(|p| command.contains(p))
}
```

## 子任务

1. **基础镜像构建** (4h)
   - 选择基础镜像
   - 安装系统依赖
   - 配置环境变量

2. **生信工具安装** (4h)
   - 安装 FastQC
   - 安装 HISAT2 / STAR
   - 安装 featureCounts
   - 安装 R 包 (DESeq2, edgeR)
   - 安装 Python 包

3. **安全机制实现** (4h)
   - 非 root 用户执行
   - 命令过滤机制
   - 资源限制配置
   - 网络隔离

4. **与 Tauri 集成** (2h)
   - Docker API 封装
   - 执行结果解析
   - 错误处理

5. **测试和文档** (2h)
   - 编写测试用例
   - 编写使用文档

## 依赖

- T001: 项目基础架构搭建

## 输出物

- Dockerfile
- 执行环境配置文档
- 安全机制实现代码

## 风险

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| Tauri + Docker 兼容性 | 高 | 提前测试集成方案 |
| 镜像体积过大 | 中 | 多阶段构建，精简依赖 |

## 备注

执行环境是核心能力，需要优先验证 Docker + Tauri 的可行性。
