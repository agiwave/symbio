//! Home 插件的请求/响应 schema
//!
//! 归属规则：这四组契约只被 HomePlugin 消费，故定义在本插件内部，core 不承载。
//!
//! **每条臂的载荷都必须是一个类型**：`json!({...})` 现拼的响应没有结构体可指，
//! 前端那份抄本就没有任何守卫能读它（`protocol-mirror-audit` D 组按结构体名取字段，
//! 找不到类型时是**报错**而不是跳过）。见 [ADR-112](../../../../docs/010-adr/112.md) 决策 2。

pub mod home_reload {
    //! home/reload 路由的请求/响应 schema（对应 HomePlugin::route 的 reload 分支）
    //!
    //! 前端镜像在 `tauri/src/schemas/home.ts` 的 **同名 namespace** `home_reload` 里——
    //! 一个文件四个模块，namespace 就是模块名，两侧类型名逐字相同。
    //!
    //! Corresponding Frontend: tauri/src/schemas/home.ts

    use serde::{Deserialize, Serialize};

    /// home/reload 请求
    ///
    /// - `homedir`：可选，要切换到的目标 homedir（绝对路径或 `~` 前缀）。
    ///   若不传，仅重新加载当前 homedir 的 config（用于 bootstrap 恢复后的二次重建）。
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Request {
        /// 目标 homedir（可选）
        #[serde(default)]
        pub homedir: Option<String>,
    }

    /// home/reload 响应
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        /// 切换前的 homedir
        pub old_homedir: String,
        /// 切换后的 homedir
        pub new_homedir: String,
        /// 重新构造的子插件数量
        pub reloaded_plugins: usize,
        /// 是否实际发生了 homedir 切换（false 表示仅重新加载）
        pub homedir_changed: bool,
        /// bootstrap 文件路径
        pub bootstrap_path: String,
    }
}

pub mod home_get_workspace {
    //! home/get_workspace 路由的响应 schema
    //!
    //! Corresponding Frontend: tauri/src/schemas/home.ts

    use serde::{Deserialize, Serialize};

    /// 获取工作区响应
    ///
    /// 「未配置工作区」用**空串**表达而不是报错：前端据此显示「未选择工作区」。
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Response {
        pub workdir: String,
        pub expanded_path: String,
        #[serde(default)]
        pub recent_workspaces: Vec<String>,
    }
}

pub mod home_set_workspace {
    //! home/set_workspace 路由的请求/响应 schema
    //!
    //! Corresponding Frontend: tauri/src/schemas/home.ts

    use serde::{Deserialize, Serialize};

    /// 切换工作区请求
    #[derive(Debug, Clone, Deserialize)]
    pub struct Request {
        /// 目标工作区（绝对路径或 `~` 前缀，展开后落进 `work.workdir`）
        pub path: String,
    }

    /// 切换工作区响应
    ///
    /// **没有 `status` 格**：成功已经由「这条臂返回 Ok」表达，再带一个恒为
    /// `"success"` 的字段是第二次说同一件事（它零消费方，见 [ADR-112](../../../../docs/010-adr/112.md) 决策 2）。
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        /// 落进配置的原值（可能是 `~` 前缀形态）
        pub workdir: String,
        /// 展开后的绝对路径
        pub expanded_path: String,
        /// 最近使用列表（本条已置顶，最多 10 条）
        pub recent_workspaces: Vec<String>,
    }
}

pub mod home_get_homedir {
    //! home/get_homedir 路由的响应 schema
    //!
    //! Corresponding Frontend: tauri/src/schemas/home.ts

    use serde::{Deserialize, Serialize};

    /// 系统目录信息
    ///
    /// **没有 `default_homedir` 格**：它的 `path` 与 `homedir` 同源同值，
    /// 且全仓零消费方——一份事实的第二个名字（[ADR-112](../../../../docs/010-adr/112.md) 决策 2）。
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        /// 当前系统根目录（`HomedirRegistry::get()`）
        pub homedir: String,
        /// 引导文件位置（位于用户主目录下）
        pub bootstrap_path: String,
    }
}
