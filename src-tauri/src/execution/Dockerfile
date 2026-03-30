# Symbio 生信分析执行环境
# 基于 Ubuntu 22.04，包含常用生信工具

FROM ubuntu:22.04

# 避免交互式提示
ENV DEBIAN_FRONTEND=noninteractive

# 安装基础工具
RUN apt-get update && apt-get install -y \
    wget \
    curl \
    git \
    unzip \
    python3 \
    python3-pip \
    python3-venv \
    r-base \
    && rm -rf /var/lib/apt/lists/*

# 安装 Python 生信库
RUN pip3 install --no-cache-dir \
    numpy \
    pandas \
    scipy \
    matplotlib \
    seaborn \
    scanpy \
    anndata \
    biopython

# 安装 R 生信包
RUN R -e "install.packages(c('DESeq2', 'edgeR', 'limma', 'ggplot2'), repos='https://cloud.r-project.org/')"

# 安装 FastQC
RUN apt-get update && apt-get install -y fastqc && rm -rf /var/lib/apt/lists/*

# 设置工作目录
WORKDIR /workspace

# 创建非 root 用户
RUN useradd -m -s /bin/bash analyst && \
    chown -R analyst:analyst /workspace

# 切换到非 root 用户
USER analyst

# 默认命令
CMD ["/bin/bash"]
