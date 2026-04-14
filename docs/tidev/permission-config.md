# TiDev 权限配置说明

TiDev 通过 `~/.config/tidev/config.toml` 的 `permissions` 部分控制不同模式下的默认工具访问权限。

## 1. 配置位置

配置文件默认路径：

- `~/.config/tidev/config.toml`

## 2. 模式与默认行为

TiDev 当前支持两个会话模式：

- `plan`：规划模式，默认只允许只读与查询类型工具
- `build`：构建模式，默认允许所有权限类型的工具

这两个模式的默认权限可以通过 `permissions` 配置自定义。

## 3. 配置格式

在配置文件中添加 `permissions` 表格：

```toml
permissions = {
  plan = {
    read = true
    search = true
    session = true
    write = false
    edit = false
    execute = false
  }
  build = {
    read = true
    search = true
    session = true
    write = true
    edit = true
    execute = true
  }
}
```

### 说明

- `read`：允许读取文件、获取文件内容等只读工具
- `search`：允许搜索类工具，如 `grep`、`glob`、`websearch` 等
- `write`：允许写文件、创建文件、删除文件等写操作
- `edit`：允许编辑文件、代码修改工具
- `execute`：允许执行命令、运行脚本、调用外部工具
- `session`：允许与会话相关的工具，例如 `skill`、会话状态管理等

## 4. 推荐默认值

TiDev 内置默认值为：

- `plan` 模式：`read`、`search`、`session` 为 `true`，`write`、`edit`、`execute` 为 `false`
- `build` 模式：所有字段为 `true`

如果你不在配置文件中声明 `permissions`，TiDev 会自动使用该默认行为。

## 5. 自定义示例

### 只允许 `plan` 模式查询、`build` 模式禁用执行

```toml
permissions = {
  plan = { read = true, search = true, session = true, write = false, edit = false, execute = false }
  build = { read = true, search = true, session = true, write = true, edit = true, execute = false }
}
```

### `plan` 模式更严格，只允许只读

```toml
permissions = {
  plan = { read = true, search = false, session = true, write = false, edit = false, execute = false }
  build = { read = true, search = true, session = true, write = true, edit = true, execute = true }
}
```

## 6. 注意事项

- `permissions` 配置只对 TiDev 本地工具与已连接 MCP 工具的默认可用性生效。
- 如果你禁用了某个权限类别，当前模式下对应权限类型的工具将不再显示为可执行。
- 如果当前模式允许某个权限类别，则该类别下的工具会直接执行，无需额外审批。
