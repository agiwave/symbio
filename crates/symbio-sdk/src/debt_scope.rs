//! 「端口的债全部还清才欠」的**作用域**（D-446）。
//!
//! 为什么住 `symbio-sdk` 而不是门面：这是**契约语义**，与 [`crate::port_stamp`]
//! 同一档——判据在宿主与环那一侧，插件作者只是把它写进自己那一行债。
//! 门面（`symbio-plugin-api`）只转出类型，不自己实现（`facade-loc-audit` 的 FA-001）。
//!
//! 形状只有两格：端口名是必有的那一半，`ids` 是可选的那一半。
//! - `None` —— **整条端口**：该端口上任何未收束债都挡住本行。最保守的读法，
//!   也是 `when_no_open("tool")` 一直以来的语义；
//! - `Some(ids)` —— 只认**这一批债 id**：同端口上别的债（上一轮遗留、重试前的旧
//!   attempt、别的请求的调用）不挡。
//!
//! 为什么要后者：一笔迟归 / 卡住的调用债会把整条端口长期占住，于是这一轮明明已经
//! 收束，续跑还在等一条与它无关的旧债。**集合是什么只有申报方知道**（它自己按
//! `calls[]` 派生的那些 id），机制不知道也不该猜——所以集合由申报方给，环只按
//! `port == 端口 && id ∈ 集合` 求值。

/// 一个「端口干净才欠」谓词的作用域。
///
/// 两格**私有**、只经读口暴露：CS-009 数的是 `pub struct` 的公共字段（内核数据面
/// 宽度只许降），而这里的两格是**一个判据的两半**，没有独立变化的余地——放开字段
/// 只会让调用方各拼一半，读口能保证「端口 + 集合」始终一起被看到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoOpenScope {
    port: String,
    ids: Option<Vec<String>>,
}

impl NoOpenScope {
    /// 整条端口。
    pub fn whole(port: impl Into<String>) -> Self {
        Self {
            port: port.into(),
            ids: None,
        }
    }

    /// 只认这批债 id。
    pub fn among(port: impl Into<String>, ids: Vec<String>) -> Self {
        Self {
            port: port.into(),
            ids: Some(ids),
        }
    }

    /// 端口名（数据；环不解释）。
    pub fn port(&self) -> &str {
        &self.port
    }

    /// 债 id 集合；`None` = 整条端口。
    pub fn ids(&self) -> Option<&[String]> {
        self.ids.as_deref()
    }

    /// 这笔债在不在作用域里（端口那一半由调用方先判）。
    pub fn covers(&self, id: &str) -> bool {
        self.ids
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|mine| mine == id))
    }
}

impl From<&str> for NoOpenScope {
    fn from(port: &str) -> Self {
        Self::whole(port)
    }
}

impl From<String> for NoOpenScope {
    fn from(port: String) -> Self {
        Self::whole(port)
    }
}
