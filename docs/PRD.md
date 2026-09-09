# WinTerminalP 产品需求文档

| 项目 | 内容 |
|---|---|
| 文档版本 | v0.2 |
| 状态 | Release Candidate（MVP 已实现） |
| 更新日期 | 2026-08-30 |
| 产品形态 | Windows Terminal 原生界面的无界面键盘与窗格鼠标控制器 |
| 核心语言 | Rust |
| 首发平台 | Windows 10/11 x64 |

## 1. 产品摘要

WinTerminalP 不再实现新的终端窗口、终端渲染器或桌面界面。产品直接复用用户已经安装的 Windows Terminal，只提供一个常驻、无界面的 Rust 控制器，将 tmux 风格的 `Ctrl+B` 两段式快捷键转换为 Windows Terminal 原生 Action。

Windows Terminal 继续负责标签页、窗格、Shell、字体、主题、复制粘贴、IME 和终端渲染。WinTerminalP 只负责：

- 判断当前前台是否为 Windows Terminal。
- 识别 `Ctrl+B` Prefix 及第二个按键。
- 将操作发送回触发 Prefix 时的同一个 Windows Terminal 窗口。
- 在不接管 pane tree 的前提下，把原生可见分隔线拖动转换为 `resizePane`。
- 安装和卸载产品自有的 Action 与隐藏桥接键。
- 提供诊断、配置和安全恢复能力。

## 2. 背景与架构修正

v0.1 使用 Tauri、React 和 xterm 自绘了完整终端应用。该方案重复实现了用户已经认可的 Windows Terminal 界面，视觉、交互和兼容性都偏离目标。

v0.2 的核心原则是：

> 不替代 Windows Terminal，只增强它的键盘控制能力。

旧 Tauri/xterm 原型已在 v0.2 重写时移除，不再作为构建入口。

## 3. 产品目标

### 3.1 P0 目标

- 启动控制器时直接打开或复用 Windows Terminal，而不是出现新的应用界面。
- 仅在 Windows Terminal 位于前台时拦截 `Ctrl+B`。
- 支持四向创建窗格、四向聚焦、四向调整尺寸。
- 支持在原生窗格边界按住左键拖动比例，并显示标准横向/纵向缩放光标。
- 支持新建/切换标签页、关闭窗格、临时放大窗格和重命名标签页。
- 支持在 TOML 中自定义 Prefix、超时时间和全部动作键位，无需重新编译或重装 Action Bridge。
- 对其他应用、普通键盘输入和第三方注入事件保持透明。
- 安装前检测按键冲突，修改 Windows Terminal 配置前创建原字节备份。
- 卸载时只移除仍属于 WinTerminalP 的配置，不覆盖用户后续修改。
- 所有核心状态机和配置变更具备自动化测试。

### 3.2 P1 目标
- 安装包、桌面快捷方式和登录启动选项。
- 稳定版、预览版、Canary、非商店版和 Portable 的完整安装体验。
- 可选的轻量托盘入口，仅用于启停和诊断，不承载终端界面。
- 签名发布和自动更新。

### 3.3 非目标

- 不实现 Tauri/WebView/React 界面。
- 不内嵌 xterm，也不直接管理 ConPTY 或 Shell 进程。
- 不替代 Windows Terminal 的主题、字体、标签栏或窗格渲染。
- 不兼容 tmux 协议、插件或配置文件。
- 不承诺关闭 Windows Terminal 后继续保留进程；Windows Terminal 没有 tmux server/session daemon。
- 不向后台记录终端内容或用户键入的字符。
- 不使用 `wt -w 0` 作为动作总线，因为它表示最近使用窗口，而不是触发 Prefix 时捕获的窗口。
- 不持久化、重建或声称读取 Windows Terminal 的真实 pane tree；UI Automation 几何只是可丢弃的可见状态快照。

## 4. 目标用户与场景

目标用户是习惯 Windows Terminal，但希望用 tmux 式 Prefix 高速操作标签页和窗格的 Windows 开发者、运维人员和高级用户。

典型流程：

1. 用户运行 `winter`，或直接双击 `winterd.exe`。
2. 控制器复用或打开 Windows Terminal。
3. 用户按 `Ctrl+B`，再按 `Shift+Right`，当前窗格向右分割。
4. 用户按 `Ctrl+B`，再按方向键，在窗格间移动焦点。
5. 用户按 `Ctrl+B`，再按 `Ctrl+方向键`，调整当前窗格尺寸。
6. 用户关闭 Windows Terminal；控制器仍不监听或影响其他应用。
7. 用户按 `Ctrl+B`、`Q` 退出控制器。

## 5. 功能需求

### FR-001 原生窗口复用

- 产品不得创建新的终端渲染窗口。
- 启动入口只启动 Rust 控制器和 Windows Terminal。
- Windows Terminal 的现有 Profile、主题、字体和 Shell 配置保持有效。
- 启动 Windows Terminal 使用官方 `wt.exe`；窗格动作不通过“最近窗口”猜测目标。

### FR-002 前台窗口限制

- Prefix 只在前台进程精确识别为 `WindowsTerminal.exe` 时生效。
- 捕获目标至少包含 HWND、PID、进程启动标记和 Terminal 通道。
- 派发 Action 前再次验证前台窗口仍为同一目标。
- 验证失败时丢弃 Action，不得转发到另一个 Terminal 窗口。

### FR-003 Prefix 状态机

- 默认 Prefix：`Ctrl+B`。
- Prefix 与动作键位均可配置；现有 schema 1 三字段配置缺少新字段时继续使用完整默认表，并在内存中迁移到 schema 2，不隐式改写原文件。
- 默认超时：1500 ms；合法范围 250–5000 ms。
- Prefix 超时后取消，不补发延迟的 `Ctrl+B`。
- 未绑定的第二键默认被吞掉并取消 Prefix。
- `Escape` 取消 Prefix。
- Prefix 后再次按 `B`，向当前终端发送原始 Ctrl+B 字符。
- 已吞掉的 keydown，其自动重复和对应 keyup 也必须吞掉。
- 自身或第三方注入的按键不得激活 Prefix。
- `Alt+Tab`、`Alt+F4`、`Ctrl+Esc` 等系统切换键应取消 Prefix 并正常透传。

### FR-004 窗格操作

| 操作 | Windows Terminal Action | 要求 |
|---|---|---|
| 四向分屏 | `splitPane` | 使用 `splitMode: duplicate`，新窗格位于指定方向 |
| 四向聚焦 | `moveFocus` | 使用 Windows Terminal 原生几何导航 |
| 四向缩放 | `resizePane` | 使用 Windows Terminal 原生步长 |
| 鼠标拖动缩放 | UI Automation + `resizePane` | 只命中相邻可见 `TermControl` 的分隔线，按局部父分栏约 5% 步长派发 |
| 关闭窗格 | `closePane` | 沿用 Windows Terminal 对最后窗格/标签页的行为 |
| 临时放大 | `togglePaneZoom` | 再次执行恢复 |

当前目录继承依赖 Shell Integration 向 Windows Terminal 报告 CWD。`winter install` 会在 PowerShell Profile 中安装受管的 `OSC 9;9` 提示符包装（带备份、幂等、可卸载），使 `splitPane`（`splitMode: duplicate`）与 `duplicateTab` 继承活动窗格目录；无法获取时遵循 Windows Terminal 自身行为。

### FR-005 标签页操作

- 新建标签页。
- 切换上一个或下一个标签页，且不显示 MRU 切换浮层。
- 使用 `0`–`9` 激活零基索引标签页。
- 打开当前标签页的原生重命名输入框。

### FR-006 Action Bridge

- Action 使用 `User.WinTerminalP.*` 命名空间。
- Action fragment 安装在当前用户 Windows Terminal fragment 目录。
- fragment 只包含 Action，不包含按键。
- 每个 Terminal 通道的 `settings.json` 只加入产品自有隐藏桥接键。
- 桥接键只使用本机真实验证可靠的 F13、F14、F15、F18–F24，并配合不常见修饰键组合；F16/F17 因 Stable 1.24 实测派发不可靠而禁用。
- Action、ID 与桥接键必须一一对应且通过测试检查重复。

### FR-007 安装与卸载

- 自动发现已经初始化且存在 `settings.json` 的 Terminal 通道。
- JSONC 修改必须保留注释、顺序、缩进、换行和尾逗号风格。
- 写入前保存原始字节、SHA-256 和安装 manifest。
- 写入前重新读取源文件并执行哈希 CAS；并发修改时中止。
- 使用同目录临时文件和原子替换。
- 同一次安装中若后续目标失败，按逆操作栈回滚已经写入的目标与 fragment；回滚前仍执行 CAS，不覆盖并发发生的用户修改。
- 同 ID 异义或同桥接键异义视为冲突，默认停止，不静默覆盖。
- 卸载只删除 ID、按键和语义仍与 manifest 一致的托管项。
- 用户修改过的托管项必须保留并报告。

### FR-008 控制器生命周期

- 同一 Windows 用户会话只运行一个控制器实例。
- Release 版控制器不显示额外控制台窗口。
- 控制器运行态必须与管理员 Windows Terminal 处于相同完整性级别：未提权的 `winter run`、`winter launch` 和 `winterd.exe` 通过 UAC 自重启，核心拒绝低完整性运行。
- `Ctrl+B`、`Q` 可退出控制器，但不关闭 Windows Terminal。
- Hook 必须有消息循环，并通过 RAII 保证退出时卸载。
- 键盘与可选鼠标 Hook 必须共享同一生命周期；非 Terminal、非分隔线以及 injected 鼠标事件全部透传。
- Hook 回调不得执行文件 IO、进程启动或阻塞等待。
- Action 使用有界队列发送给工作线程；队列满时丢弃并返回诊断，不阻塞系统输入。
- UI Automation 必须运行在独立 COM observer/worker 线程，Hook 只读取内存几何快照。

### FR-009 配置和诊断

- 配置默认位于 `%LOCALAPPDATA%\WinTerminalP\config.toml`。
- 配置带 `schema_version`，当前版本为 2；兼容旧 schema 1，其他版本、未知字段和越界值必须报错。
- `winter config` 输出文件路径和完整有效配置，`winter config --edit` 使用记事本打开配置。
- Shortcut 使用规范化 chord 字符串；未知动作、重复 chord、系统保留键、无修饰普通 Prefix 和禁用退出键必须拒绝启动。
- 未写出的动作继承默认值，值为 `disabled` 的可选动作不注册；保存后重启控制器生效。
- `[mouse_resize]` 可关闭鼠标功能，并校验分隔线命中扩展与几何刷新周期的安全范围。
- `doctor` 输出 Terminal 检测、fragment、keybinding 与配置状态；控制器实际权限在运行入口和核心启动时分别校验。
- 隐藏控制器启动失败时只记录必要错误，不记录按键和终端内容。

## 6. 默认快捷键

| 按键序列 | 操作 |
|---|---|
| `Ctrl+B`，方向键 | 聚焦对应方向窗格 |
| `Ctrl+B`，`Shift+方向键` | 向对应方向分屏 |
| `Ctrl+B`，`Ctrl+方向键` | 向对应方向调整尺寸 |
| `Ctrl+B`，`C` | 新建标签页 |
| `Ctrl+B`，`N` / `P` | 下一个 / 上一个标签页 |
| `Ctrl+B`，`0`–`9` | 激活对应零基标签页 |
| `Ctrl+B`，`X` | 关闭活动窗格 |
| `Ctrl+B`，`Z` | 放大 / 恢复活动窗格 |
| `Ctrl+B`，`,` | 重命名标签页 |
| `Ctrl+B`，`B` | 发送原始 Ctrl+B |
| `Ctrl+B`，`Q` | 退出控制器 |
| `Ctrl+B`，`Escape` | 取消 Prefix |

## 7. 安全与隐私

- 控制器使用低级键盘 Hook，但不保存、上传或记录用户输入。
- 鼠标 Hook 只判断屏幕点是否命中缓存分隔线，不保存、上传或记录鼠标轨迹。
- 非 Windows Terminal 前台时所有按键透传。
- `SendInput` 受 UIPI 限制；控制器运行入口会自动提权，并在核心启动时再次拒绝低完整性运行，确保管理员权限 Terminal 的完整 Prefix 与鼠标功能可用。
- 产品只修改自己的 fragment、自己的配置文件及经备份的 Terminal `keybindings`。
- 不修改 `defaults.json`、`state.json`、Terminal Profile 或 Shell 配置。
- 不使用网络服务、遥测或远程控制端口。

## 8. 验收标准

- 默认构建不再包含或启动 Tauri、React、WebView 或 xterm。
- `cargo fmt --all -- --check`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings` 全部通过。
- 安装计划和卸载计划在临时 JSONC 配置上保留原内容与注释。
- 非 Terminal 前台的 Ctrl+B 不受影响。
- 两个 Terminal 窗口并存时，只操作触发 Prefix 的窗口。
- 四向 split、focus，以及 close、next/previous/switch tab 已在真实 Windows Terminal Stable 1.24 上验证。
- 四向 resize、zoom、rename 和物理键盘 Prefix 需按 `docs/TESTING.md` 做最终人工可视确认；Windows Terminal 不提供 pane tree 查询 API，自动测试不伪造该结论。
- 物理边界拖动需在横向、纵向和嵌套分栏上做最终人工可视确认；合成鼠标输入按安全策略不参与验收。
- 配置冲突、前台切换、队列满和注入失败均有明确诊断。
- Stable 1.24 在当前开发机完成至少一次安装、运行和恢复验证。

## 9. 官方能力依据

- [Windows Terminal Actions](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions)
- [Windows Terminal Panes](https://learn.microsoft.com/en-us/windows/terminal/panes)
- [Windows Terminal command line arguments](https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments)
- [JSON Fragment Extensions](https://learn.microsoft.com/en-us/windows/terminal/json-fragment-extensions)
- [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
- [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
- [UI Automation overview](https://learn.microsoft.com/en-us/windows/win32/winauto/entry-uiauto-win32)
- [Action fragment implementation](https://github.com/microsoft/terminal/pull/16185)
