//! 节点访问位：`r` 读 / `w` 写 / `l` 列 / `t` 遍历的紧凑表示、解析与判定。

use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ==================== 访问位 ====================

/// 节点访问位：`r` 读 / `w` 写 / `l` 列表 / `t` 树状遍历。
///
/// 线上表示为紧凑字符串（按 `r` `w` `l` `t` 顺序拼接，缺位即无该能力）：
/// `"rl"` = 可读 + 可列；`"rw"` = 可读可写；`"wlt"` = 可写可列可遍历；`""` = 不可访问。
///
/// 机制与消费者**只认访问位**，不做任何按类型的特判——这是 VDFS 保持通用的根基。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct VdfsAccess {
    /// `r`：可读内容
    pub read: bool,
    /// `w`：可写内容
    pub write: bool,
    /// `l`：可列出直接子节点
    pub list: bool,
    /// `t`：可树状递归遍历
    pub traverse: bool,
}

impl VdfsAccess {
    pub const NONE: Self = Self {
        read: false,
        write: false,
        list: false,
        traverse: false,
    };
    /// 只读文件（`r`）
    pub const READ: Self = Self {
        read: true,
        ..Self::NONE
    };
    /// 可读写文件（`rw`）
    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        ..Self::NONE
    };
    /// 只读目录（`l`）
    pub const LIST: Self = Self {
        list: true,
        ..Self::NONE
    };
    /// 只读目录 + 树状遍历（`lt`）
    pub const LIST_TRAVERSE: Self = Self {
        list: true,
        traverse: true,
        ..Self::NONE
    };
    /// 可读写目录 + 树状遍历（`lwt`）
    pub const LIST_WRITE_TRAVERSE: Self = Self {
        list: true,
        write: true,
        traverse: true,
        ..Self::NONE
    };

    /// 目录（可选择可写 / 可遍历）
    pub fn dir(write: bool, traverse: bool) -> Self {
        Self {
            list: true,
            write,
            traverse,
            read: false,
        }
    }

    /// 文件（可选择可写）
    pub fn file(write: bool) -> Self {
        Self {
            read: true,
            write,
            list: false,
            traverse: false,
        }
    }

    /// 紧凑表示（`r` `w` `l` `t` 顺序）
    pub fn flags(&self) -> String {
        let mut s = String::with_capacity(4);
        if self.read {
            s.push('r');
        }
        if self.write {
            s.push('w');
        }
        if self.list {
            s.push('l');
        }
        if self.traverse {
            s.push('t');
        }
        s
    }

    /// 从紧凑表示解析（忽略顺序与未知字符）
    pub fn parse(s: &str) -> Self {
        let mut a = Self::NONE;
        for c in s.chars() {
            match c {
                'r' | 'R' => a.read = true,
                'w' | 'W' => a.write = true,
                'l' | 'L' => a.list = true,
                't' | 'T' => a.traverse = true,
                _ => {}
            }
        }
        a
    }

    /// 是否含全部给定位
    pub fn contains(&self, other: Self) -> bool {
        (!other.read || self.read)
            && (!other.write || self.write)
            && (!other.list || self.list)
            && (!other.traverse || self.traverse)
    }

    /// 是否无任何能力
    pub fn is_none(&self) -> bool {
        !self.read && !self.write && !self.list && !self.traverse
    }
}

impl Serialize for VdfsAccess {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.flags())
    }
}

impl<'de> Deserialize<'de> for VdfsAccess {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(d)?))
    }
}
