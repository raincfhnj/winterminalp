# WinTerminal++ 架构设计

本文档描述本地多窗格终端 MVP 的实现架构。产品范围以 docs/PRD.md 为准，开发顺序以 docs/IMPLEMENTATION_PLAN.md 为准。

## 1. 架构结论

- Tauri v2 提供 Windows 桌面窗口、WebView、IPC 和打包能力。
- React/TypeScript 负责界面与终端画布。
- Rust Core 负责 Session、Tab、Pane、Layout、Profile 和 PTY 生命周期。
- 领域层不依赖 Tauri 或 Windows API。
- 终端适配层通过 portable-pty 使用 Windows ConPTY。
- P0 的 PTY 随 Tauri Core 退出；P1 才由独立 winterminald 持有持久会话。

## 2. 进程和线程

    Windows
    └── WinTerminal++ Tauri Process
        ├── Rust Core
        │   ├── Main Thread / Tauri Runtime
        │   ├── AppState
        │   └── one blocking reader thread per PTY
        ├── WebView2 Process
        │   ├── React
        │   └── one xterm instance per mounted Pane
        └── Child Processes
            ├── powershell.exe
            ├── cmd.exe
            └── wsl.exe

终端读取使用专用线程，因为 portable-pty 暴露阻塞 Reader。读取线程只负责读取有界数据块并投递 TerminalEvent，不持有 AppModel 锁。

## 3. 模块

### 3.1 Rust

| 模块 | 职责 | 禁止事项 |
|---|---|---|
| domain | 会话、标签页、布局树、焦点、缩放、快照 | 不依赖 Tauri、PTY 或文件系统 |
| terminal | Profile、PTY、输入输出、resize、终止 | 不修改布局状态 |
| commands | 参数校验、状态编排、错误映射 | 不包含布局算法 |
| error | 稳定 IPC 错误结构 | 不泄露敏感系统信息 |
| lib | 组合 AppState、注册 Commands、启动应用 | 不承载业务算法 |

### 3.2 TypeScript

| 模块 | 职责 |
|---|---|
| types | Rust IPC 快照和事件的镜像类型 |
| services | typed invoke、Channel 和错误归一化 |
| components | 标签栏、递归布局、终端窗格和状态栏 |
| hooks | Prefix 键状态机、窗口尺寸和命令调度 |
| preview | 非 Tauri 环境的只读演示快照 |

## 4. 权威状态

AppModel 是布局与会话的唯一权威状态源。

    User intent
        ↓
    typed invoke
        ↓
    Tauri Command
        ↓
    lock AppModel
        ↓
    validate and mutate
        ↓
    revision + 1
        ↓
    return AppSnapshot
        ↓
    React replaces cached snapshot

前端可以缓存快照，但不得：

- 生成真实 Session、Tab 或 Pane ID。
- 独立修改 LayoutNode。
- 假设失败的 Command 已经生效。
- 使用本地状态覆盖更高 revision 的 Rust 快照。

## 5. 布局模型

LayoutNode 是二叉树：

    Pane

或：

    Split
    ├── axis: row | column
    ├── ratio
    ├── first
    └── second

归一化坐标以左上角为原点：

- row 沿 x 轴分割左右区域。
- column 沿 y 轴分割上下区域。
- ratio 表示 first 子树占当前区域的比例。

焦点移动先将所有叶子转换为 Rect，再使用方向半平面、边缘距离、投影重叠、中心距离和稳定 ID 选择目标。

关闭叶子时用兄弟节点替换父 Split。此操作必须在单次锁内完成，不能向前端暴露空 Split。

## 6. 终端生命周期

    Pane created
        ↓
    status = starting
        ↓
    start_terminal / start_or_attach
        ├── create PTY and child
        ├── store writer/master/child
        ├── start reader thread
        └── attach Channel sink
        ↓
    status = running
        ↓
    output / input / resize
        ↓
    exited | close | restart

同一个 Pane 只允许一个 PTY。React 组件重新挂载时，start_terminal 只替换输出订阅，不重复启动进程。

关闭 Pane 或 Tab 的 Command 先完成领域变更，再根据 removedPaneIds 终止对应 PTY。终止失败需要返回可见错误，但不得把已经从布局中删除的 Pane 静默恢复为不一致状态；集成层应记录错误并确保 TerminalManager 最终移除 handle。

## 7. 锁和并发

- AppModel 使用单个 Mutex，布局操作持锁时间短且不执行 IO。
- TerminalManager 使用内部锁保护 Pane 到 TerminalHandle 的映射。
- 每个 writer 和可变 PTY handle 使用独立锁。
- 不在持有 AppModel 锁时启动、写入或终止进程。
- 不在持有 TerminalManager 全局映射锁时进行阻塞读取。
- 输出 sequence 在每个 Pane 内单调递增。

锁顺序固定为：

1. AppModel。
2. 释放 AppModel。
3. TerminalManager。

禁止同时持有两个模块的全局锁。

## 8. 错误策略

领域错误、终端错误和锁错误统一映射为 AppError：

    code
    message
    retryable
    paneId?
    detail?

- code 是稳定机器码。
- message 可以直接显示给用户。
- detail 用于本地诊断，不包含环境变量、完整命令或终端输出。
- 可恢复错误返回 Result，不使用 panic。
- Tauri 启动失败是进程级不可恢复错误，可以由框架入口结束应用。

## 9. 安全

- capabilities 仅包含主窗口所需 core 权限。
- 不安装或授权通用 shell 执行插件。
- Profile ID 在 Rust 中映射固定程序和参数。
- 前端不能提供任意 executable。
- cwd 由 Rust 校验为存在的目录；无效值安全回退。
- CSP 不允许远程脚本或 unsafe-eval。
- withGlobalTauri 关闭。
- 默认无遥测。

## 10. P1 演进

P1 把以下模块移动到 winterminald：

- AppModel。
- TerminalManager。
- PTY reader threads。
- 输出 ring buffer。

Tauri Core 从直接函数调用改为本地命名管道客户端。Domain 和 Terminal 的公共接口保持不变，Commands 仅替换应用服务适配器。

    React
       ↓ Tauri IPC
    Tauri Core
       ↓ versioned named-pipe protocol
    winterminald
       ↓
    ConPTY

P1 不通过公网端口连接，命名管道 ACL 限制为当前 Windows 用户。

## 11. 架构验证

- cargo test 验证纯 Domain。
- cargo clippy 验证 Rust 边界和错误处理。
- Vitest 验证快捷键和递归布局。
- Tauri debug build 验证 IPC 类型和命令注册。
- 真实 Tauri 窗口验证 ConPTY、WebView2、IME 和 resize。
- 关闭 Pane 后检查终端 handle 和前端订阅均已释放。
