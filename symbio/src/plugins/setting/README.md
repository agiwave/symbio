# Setting 插件

系统级配置读写插件：为其他插件与前端提供统一的配置存取入口。

## 路由

| Path | 说明 |
|------|------|
| `setting/get` | 读取配置项 |
| `setting/set` | 写入配置项 |

## 机制

- 配置持久化在主目录（与 home 的 config.yaml 体系协同，详见 `../home/README.md`）。
- 统一实体协议下另有 `entities/settings` 读写入口（见 `docs/design/entity-management-mechanism.md`）。

## 关联

- 全局配置：`../home/README.md`
- 统一实体协议：`docs/design/entity-management-mechanism.md`
