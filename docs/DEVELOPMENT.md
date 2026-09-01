# WinTerminal++ 开发指南

## 环境

- Windows 10/11 x64。
- Rust stable，最低 Rust 1.85，项目当前使用 2024 Edition。
- Windows Terminal 1.21+。
- MSVC C++ Build Tools。

项目根目录是 Rust 控制器。旧 Tauri/xterm 原型位于 `archive/tauri-xterm-prototype/`，不参与默认构建。

## 常用命令

```powershell
cargo build
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

开发期以前台模式运行控制器：

```powershell
cargo run --bin wter -- run --no-launch
```

安装计划必须先只读查看：

```powershell
cargo run --bin wter -- plan
cargo run --bin wter -- doctor
```

确认无冲突后安装：

```powershell
cargo run --bin wter -- install
```

Release 隐藏控制器：

```powershell
cargo build --release --bins
target\release\winterminald.exe
```

`winterminald.exe` 可直接双击：集成已安装时，它在后台启动控制器并打开原生 Windows Terminal，不创建 WinTerminal++ 界面。`wter.exe` 是推荐的短命令入口，`winterminal.exe` 保留为兼容别名：

```powershell
target\release\wter.exe
target\release\wter.exe doctor
target\release\wter.exe install
target\release\wter.exe uninstall
```

若通过 `cargo install --path . --bin wter` 安装到已经加入 `PATH` 的 Cargo bin 目录，可在任意目录直接运行 `wter`。

查看或编辑用户快捷键：

```powershell
wter config
wter config --path
wter config --edit
```

Shortcut 字段采用 `action_name = "modifier+key"`；缺失项继承默认值。配置解析、重复键与系统保留键校验必须留在 `config`/`prefix`，不能放进 Hook callback。

## 代码边界

| 模块 | 职责 |
|---|---|
| `model` | 方向、动作、Terminal 通道和窗口身份 |
| `config` | 应用 TOML schema、兼容默认值与 Shortcut 校验 |
| `keymap` | Action ID、WT command 和隐藏 chord 的唯一表 |
| `prefix` | KeyChord 契约、默认动作表和不依赖 Win32 的纯 Prefix 状态机 |
| `pane_layout` | 纯几何分隔线推导、命中测试和拖动步长状态 |
| `platform/windows` | 统一输入 Hook、UI Automation、前台识别、SendInput、单实例和启动 |
| `integration` | Terminal 发现、fragment、JSONC、备份和 manifest |
| `controller/keyboard` | Win32 虚拟键与物理 modifier 的无 IO 规范化 |
| `controller/desktop` | 前台身份与可丢弃窗格几何快照，不拥有布局 |
| `controller` | Prefix/拖动 reducer、有界工作队列和 dispatcher 编排 |
| `bin` | CLI 与隐藏 daemon 入口 |

## Rust 规范

- 使用 `rustfmt` 默认格式。
- Clippy 以 `-D warnings` 运行。
- 可恢复错误返回 `Result`，不在生产输入上 `unwrap` 或 `expect`。
- 所有 HANDLE、Hook 和 Mutex 必须使用 RAII。
- `unsafe` 块必须小且带安全不变量说明。
- Hook callback 禁止 IO、阻塞和进程启动。
- UI Automation 只能在 observer/worker 线程调用，不能进入 Hook callback。
- 不保存或自行修改 pane tree；`PaneLayout` 只能由最新原生矩形重新推导。
- 测试 fixture 可以使用 `expect`，生产路径不使用。

## 配置安全

- 开发测试优先使用临时目录中的 JSONC fixture。
- 不直接格式化或重写真实 `settings.json`。
- 真实安装前记录目标绝对路径、原始哈希和备份路径。
- 不修改 `defaults.json` 或 `state.json`。
- 测试卸载时仅操作 `User.WinTerminalPP.*` 命名空间。
