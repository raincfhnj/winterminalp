# WinTerminal++ v0.2 实施计划

## 1. 项目概览

- 名称：WinTerminal++。
- 模式：从 Tauri/xterm 自绘终端重构为 Windows Terminal 原生增强器。
- 语言：Rust 2024 Edition。
- 平台：Windows 10/11 x64。
- 外部依赖：Windows Terminal 1.21+、Win32 Hook/Input 与 UI Automation API。
- 数据库、缓存、消息队列、HTTP API：均不需要。
- 界面：不创建；完全复用 Windows Terminal。

实施计划 skill 中的 OpenAPI、数据库和前端联调产物不适用于该本地无服务项目，分别以 CLI 契约、Action Bridge 契约和 Windows 集成说明替代。

## 2. 重构差距

| 类别 | v0.1 | v0.2 目标 | 决策 |
|---|---|---|---|
| 界面 | Tauri + React | Windows Terminal 原生界面 | 旧实现已移除 |
| 渲染 | xterm | Windows Terminal | 删除默认依赖 |
| Shell/PTY | Rust portable-pty | Windows Terminal 管理 | 不再重复管理 |
| 布局 | 自研二叉树 | Windows Terminal 原生 pane tree | 只缓存可丢弃 `TermControl` 矩形，不保存布局副本 |
| 输入 | WebView 内 Prefix | 统一键盘/鼠标 Hook，仅 WT 前台生效 | 键盘 Prefix + 原生边界拖动 |
| 动作通道 | Tauri IPC | Action fragment + 隐藏 keybinding + SendInput | 重写 |
| 配置 | 应用内 IPC | App TOML + lossless Terminal JSONC | 新增 |
| 安全边界 | WebView capability | 前台身份、UIPI、配置事务 | 重写 |

旧 Tauri/xterm 原型已在 v0.2 重写时移除，仓库只保留根 Cargo 构建。

## 3. 架构

采用小型模块化单体、双二进制：

    wter.exe（winterminal.exe 兼容别名）
      ├── install / uninstall
      ├── doctor
      ├── run（前台调试）
      └── launch

    winterminald.exe
      ├── single-instance guard
      ├── WH_KEYBOARD_LL hook thread
      ├── Prefix reducer
      └── bounded action worker
              │
              └── SendInput bridge
                       │
                       ▼
                Windows Terminal

模块依赖：

    model ← keymap ← integration
      ↑        ↑          ↑
    prefix   platform   CLI/controller

`prefix` 不依赖 Win32；`platform` 不解析配置；`integration` 不安装 Hook。

## 4. 技术选型

| 类别 | 选择 | 原因 |
|---|---|---|
| CLI | clap 4 | 稳定的 Windows 命令行解析 |
| Win32 | windows 0.62 | 官方 Rust 投影，便于 HANDLE 与 Result 封装 |
| JSONC | jsonc-parser CST | 只改目标节点并保留用户格式和注释 |
| 配置 | serde + toml | 结构化 schema 与可读配置 |
| 哈希 | SHA-256 | 安装 CAS、备份和卸载语义确认 |
| 错误 | thiserror | 可测试的结构化错误 |
| 并发 | std::sync::mpsc::sync_channel | Hook 回调使用有界非阻塞队列，UIA 位于独立线程 |
| 测试 | cargo test | reducer、桥接表、JSONC 和 Win32 边界 |
| 质量 | rustfmt + clippy | 自动化 Rust 规范 |

## 5. 共享契约

### 5.1 领域动作

```rust
enum TerminalAction {
    SendPrefixLiteral,
    SplitPane { direction: Direction },
    FocusPane { direction: Direction },
    ResizePane { direction: Direction },
    NewTab,
    NextTab,
    PreviousTab,
    ActivateTab { index: u8 },
    ClosePane,
    TogglePaneZoom,
    RenameTab,
}
```

### 5.2 目标身份

```rust
struct WindowIdentity {
    hwnd: isize,
    process_id: u32,
    process_started_at_100ns: u64,
    channel: TerminalChannel,
}
```

动作派发前必须重新获取前台目标并与四个字段比较。

### 5.3 Bridge 不变量

- 每个 `TerminalAction` 恰好对应一个 Action ID 和一个隐藏 chord。
- Action ID 位于 `User.WinTerminalPP.*`。
- fragment 不包含 keys。
- 用户 keybindings 不重复占用 chord。
- Action 发送成功只表示输入已插入，不宣称布局结果已被读取确认。
- UI Automation 几何只用于当前命中测试；它不是 Action 回执，也不能证明 pane tree 已按预期变化。

## 6. CLI 契约

| 命令 | 作用 | 是否修改系统 |
|---|---|---|
| `wter` / `wter launch` | 启动隐藏控制器和 Windows Terminal | 启动进程 |
| `wter config` | 输出完整有效 Shortcut 配置；可 `--path` 或 `--edit` | 只在首次运行时创建应用配置 |
| `wter plan` | 输出安装计划和冲突 | 否 |
| `wter install` | 安装 fragment 和当前通道 keybindings | 是，可恢复 |
| `wter uninstall` | 删除仍受管理的配置 | 是，可恢复 |
| `wter doctor` | 输出环境、安装和权限诊断 | 否 |
| `wter run` | 前台运行控制器，便于开发 | 仅进程内 |

自动发现所有已初始化的 Stable、Preview、Canary 和 Unpackaged 通道；不存在 `settings.json` 的通道被跳过，不创建伪配置。Portable 通道的 `settings.json` 与可执行文件同目录，无法从 `LOCALAPPDATA` 定位，因此不参与自动安装。

## 7. 并行任务

| Agent | 文件所有权 | 交付 |
|---|---|---|
| Prefix/Keymap | `src/keymap.rs`, `src/prefix.rs` | Action 表、桥接键、纯状态机及测试 |
| Windows Adapter | `src/platform/**` | Hook、前台识别、SendInput、单实例和启动器 |
| Integration | `src/integration/**` | fragment、JSONC 事务、备份、manifest、doctor |
| 主 Agent | Cargo、model、config、controller、CLI、docs | 契约、集成、审查和真实验证 |

并行模块禁止改动彼此所有权文件；合并后由主 Agent 统一注册模块和修正公共类型。

## 8. 配置事务

安装阶段：

1. 发现目标并读取原字节。
2. JSONC CST 解析。
3. 规范化 ID 和 chord，构造无写入计划。
4. 检测 ID、chord 和用户 unbound 冲突。
5. 保存备份和 preHash，并初始化进程内逆操作栈。
6. 写 fragment 临时文件并原子替换。
7. 只向 `keybindings` 追加缺失的托管项。
8. 写入前重读目标，preHash 不一致则中止。
9. 同目录临时文件原子替换。
10. 回读、解析、验证并写 committed manifest。
11. 向 PowerShell Profile 追加受管的 `OSC 9;9` 提示符包装（快照、备份、CAS、幂等），用于让 `splitMode: duplicate` 继承当前目录；失败不阻塞 Terminal 桥接安装，由 `doctor` 单独报告。

卸载阶段：

1. 读取 manifest 和当前配置。
2. 当前文件未变化时可恢复原始备份。
3. 文件已变化时，只删除 ID、chord 和语义仍匹配的项。
4. 用户修改项保留并报告。
5. 所有通道清理成功后删除未被修改的 fragment。

## 9. 实施阶段与状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| A | 官方能力、本机版本、仓库和风险核对 | 完成 |
| B | 旧原型无损归档、Rust 根项目 | 完成 |
| C | Prefix、Bridge、Win32、配置事务并行开发 | 完成 |
| D | CLI/controller 集成与错误归一化 | 完成 |
| E | 文档、格式化、单元测试、Clippy | 完成 |
| F | 当前 Stable 1.24 真实安装、动作、恢复验证 | 完成可自动确认部分；resize/zoom/rename/物理 Prefix 待人工可视复核 |
| G | 原生窗格几何观察、统一鼠标 Hook、边界拖动与模块拆分 | 代码与自动探针完成；物理拖动手感待人工可视复核 |

## 10. 验证门槛

### 自动化

- `cargo fmt --all -- --check`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- Bridge Action/ID/chord 完整性和唯一性。
- Prefix 超时、重复键、unknown、Escape、窗口切换、injected 输入。
- 旧配置兼容、自定义 Prefix/动作、partial override、disabled、重复与保留键拒绝。
- JSONC 注释、尾逗号、BOM、CRLF、冲突、幂等、CAS 和卸载。
- Win32 HANDLE/Hook/COM RAII、前台身份匹配与 injected 输入透传。
- 纯几何层覆盖嵌套分隔线、命中扩展、5% 步长残差与单事件动作上限。

### 真实 Windows

- Stable 和 Preview 检测结果正确。
- 安装前后设置文件均可被 Windows Terminal 加载。
- 两个 WT 窗口并存时不误投。
- 四向 split/focus/resize、close、zoom 和 tab 操作生效。
- 普通权限控制器面对提权 Terminal 时安全失败。
- 卸载后不残留托管 keybinding，用户配置可恢复。

## 11. 已知限制

- `SendInput` 是前台输入，重验 HWND 后仍存在极窄的 TOCTOU 窗口。
- UIPI 会阻止低完整性控制器向高完整性 Terminal 注入；返回值无法明确指出 UIPI 是唯一原因。
- Windows Terminal 无公开 pane tree 读取 API，动作结果只能做运行时验证，不能由控制器读取权威布局快照。
- UI Automation provider 可能瞬时失败或变慢；失败时清空命中快照，不使用陈旧几何猜测边界。
- Windows Terminal 关闭后，Shell 生命周期遵循 Terminal 自身策略，不等同 tmux detach。
- 正常运行期的多文件安装失败会回滚；进程被强杀或断电发生在多文件写入与 committed manifest 落盘之间时，当前版本没有持久化 prepared journal，需使用备份和 `doctor` 恢复。
