# WinTerminal++ 实施计划

| 项目 | 内容 |
|---|---|
| 计划版本 | v0.1 |
| 日期 | 2026-08-30 |
| 对应需求 | docs/PRD.md v0.1 |
| 开发模式 | 0→1 模块化桌面应用 |
| 并行度 | 主 Agent + 3 个子 Agent |
| 当前目标 | 可直接启动并使用的本地多窗格终端 MVP |

## 1. 本轮交付目标

本轮实现 PRD 的 M0、M1 和 M2 核心纵向切片：

- 创建 Tauri v2 + React + TypeScript + Vite 桌面工程。
- Windows 上由 Rust 通过 ConPTY 兼容 PTY 后端启动真实 PowerShell。
- Rust 是 Session、Tab、Pane、Layout 和终端生命周期的权威状态源。
- 支持向左、右、上、下创建窗格。
- 支持四向几何焦点移动。
- 支持四向调整窗格比例。
- 支持关闭、临时最大化和恢复窗格。
- 支持新建、切换和关闭标签页。
- 支持 Ctrl+B Prefix 快捷键。
- 每个窗格使用独立终端渲染器和 PTY。
- 通过 npm run tauri dev 打开真实桌面窗口。

本轮不实现：

- winterminald 持久会话 Daemon。
- UI 关闭后保留终端进程。
- 快捷键/Profile 设置页、配置持久化和标签页重命名。
- 运行中进程的可配置关闭确认；当前关闭操作立即终止对应 PTY。
- 高输出压力、中文 IME、高 DPI 和多显示器的正式验收。
- SSH Profile、远程客户端和插件系统。
- macOS、Linux 和 Windows ARM64 交付验证。
- 正式签名安装包和自动更新。

## 2. 架构决策

### 2.1 应用架构

采用模块化单体：

    React WebView
        │
        │ typed Tauri Commands + Channel
        ▼
    Tauri Rust Core
        ├── App Controller
        ├── Session/Layout Domain
        └── Terminal/PTY Adapter
                │
                ▼
             ConPTY

模块依赖只能朝向领域层：

    Tauri Commands → Application State → Domain
                                  └────→ Terminal Adapter

Domain 不依赖 Tauri、WebView、portable-pty 或 Windows API。

### 2.2 状态权威

- Rust 保存完整 AppSnapshot。
- 前端只能缓存最新快照，不能独立生成 Pane ID 或修改布局树。
- 每个 Rust 状态变更成功后 revision 加一。
- 前端调用命令后使用返回快照替换旧快照。
- 高频终端字符流不进入 AppSnapshot。

### 2.3 终端通信

- 布局、标签和生命周期操作使用 Tauri Commands。
- 终端输出使用 Tauri Channel，从 Rust 单向发送到对应前端终端。
- 终端输入使用批处理后的 Command。
- resize 使用独立 Command，并在布局尺寸变化后调用。
- 所有错误返回结构化 AppError。

### 2.4 安全边界

- WebView 不获得 shell 插件权限。
- WebView 不获得任意文件系统权限。
- 只开放注册过的应用 Commands。
- Rust 根据内置 Profile 创建进程，前端不能传任意可执行文件路径。
- CSP 只允许本地资源及 Tauri IPC 所需连接。
- withGlobalTauri 保持关闭。

## 3. 技术选型

| 类别 | 选择 | 原因 |
|---|---|---|
| 桌面框架 | Tauri v2 | Rust Core、轻量桌面壳、Windows 打包 |
| 前端 | React + TypeScript + Vite | 组件化布局和类型安全 |
| 终端渲染 | @xterm/xterm | 成熟 VT 渲染与终端交互 |
| 尺寸适配 | @xterm/addon-fit | 根据窗格 DOM 计算行列数 |
| PTY | portable-pty | Rust PTY 抽象，Windows 使用 ConPTY |
| 序列化 | serde + serde_json | Rust/TypeScript IPC 数据 |
| ID | uuid | 稳定 Session、Tab、Pane 标识 |
| Rust 错误 | thiserror + Serialize | 结构化可恢复错误 |
| Rust 并发 | std::sync + 专用读取线程 | 适配 blocking PTY IO |
| 前端测试 | Vitest | 快速 TypeScript 单测 |
| Rust 测试 | cargo test | Domain 和 PTY 适配层测试 |
| 格式化/Lint | Prettier、ESLint、rustfmt、clippy | 自动执行代码规范 |

依赖版本不在计划文档硬编码，以 package-lock.json 和 Cargo.lock 为准。

## 4. 共享领域模型

### 4.1 Rust/TypeScript 公共字段

    AppSnapshot
    ├── schemaVersion: 1
    ├── revision: number
    ├── activeSessionId: string
    └── session: SessionSnapshot

    SessionSnapshot
    ├── id: string
    ├── name: string
    ├── activeTabId: string
    └── tabs: TabSnapshot[]

    TabSnapshot
    ├── id: string
    ├── title: string
    ├── activePaneId: string
    ├── zoomedPaneId: string | null
    └── root: LayoutNode

    LayoutNode
    ├── kind: pane
    │   └── pane: PaneSnapshot
    └── kind: split
        ├── axis: row | column
        ├── ratio: number
        ├── first: LayoutNode
        └── second: LayoutNode

    PaneSnapshot
    ├── id: string
    ├── title: string
    ├── profileId: string
    └── status: starting | running | exited | error

### 4.2 方向语义

| Direction | Split 轴 | 新节点顺序 |
|---|---|---|
| left | row | new, current |
| right | row | current, new |
| up | column | new, current |
| down | column | current, new |

### 4.3 Rust 状态不变量

- 每个 Pane ID 在当前标签页中只出现一次。
- 每个 Split 始终拥有两个子节点。
- ratio 保持在 0.1 至 0.9，并受最小尺寸约束。
- activePaneId 必须指向布局中的 Pane。
- zoomedPaneId 为空或指向布局中的 Pane。
- 关闭 Pane 后提升兄弟节点。
- 关闭最后一个标签页时创建默认标签页。

## 5. Tauri IPC 契约

| Command | 参数 | 返回 | 作用 |
|---|---|---|---|
| get_app_snapshot | 无 | AppSnapshot | 获取权威状态 |
| split_pane | direction | AppSnapshot | 在活动窗格四向分割 |
| focus_pane | direction | AppSnapshot | 几何选择窗格 |
| resize_pane | direction, amount | AppSnapshot | 调整最近匹配 Split |
| close_pane | 无 | AppSnapshot | 关闭活动窗格和 PTY |
| toggle_zoom | 无 | AppSnapshot | 最大化或恢复 |
| create_tab | 无 | AppSnapshot | 新建标签页 |
| activate_tab | tabId | AppSnapshot | 激活标签页 |
| close_tab | tabId | AppSnapshot | 关闭标签页及其 PTY |
| start_terminal | paneId, rows, cols, channel | TerminalStarted | 启动并订阅终端 |
| write_terminal | paneId, data | void | 写入终端输入 |
| resize_terminal | paneId, rows, cols | void | 更新 PTY 尺寸 |
| restart_terminal | paneId, rows, cols, channel | TerminalStarted | 重启已退出终端 |

AppError：

    {
      code: string,
      message: string,
      retryable: boolean,
      paneId?: string,
      detail?: string
    }

TerminalEvent：

    {
      paneId: string,
      sequence: number,
      kind: output | exited | error,
      data?: string,
      exitCode?: number
    }

## 6. 并行任务与文件所有权

基础脚手架和共享契约由主 Agent 先完成。之后启动三条并行任务，禁止跨所有权编辑。

| Agent | 模块 | 文件所有权 | 依赖 |
|---|---|---|---|
| Agent A | Rust Layout Domain | src-tauri/src/domain/** | 仅 serde、uuid |
| Agent B | Rust Terminal Adapter | src-tauri/src/terminal/** | portable-pty、共享错误约定 |
| Agent C | React UI | src/** | 共享 TypeScript IPC 类型 |
| 主 Agent | 基础设施与集成 | package.json、配置文件、src-tauri/src/lib.rs、commands/**、docs/** | 汇总所有模块 |

### 6.1 Agent A：布局领域

交付：

- Session、Tab、Pane、LayoutNode 模型。
- 四向分割。
- 二叉树关闭与兄弟提升。
- 布局矩形计算。
- 几何焦点算法。
- 四向 resize。
- zoom 和标签页状态。
- 单元测试覆盖规则网格与不规则布局。

约束：

- 不依赖 Tauri 或 portable-pty。
- 所有失败返回 DomainError。
- 禁止使用 unwrap 处理运行时输入。

### 6.2 Agent B：终端适配器

交付：

- TerminalManager。
- PowerShell Profile。
- PTY start、write、resize、terminate 和 restart。
- blocking reader 线程和终端事件发送抽象。
- 资源释放与有界读取。
- 可测试的 Profile 解析和输入校验。

约束：

- 不修改 Layout Domain。
- 不注册 Tauri Command。
- 通过回调或 trait 暴露输出，不直接依赖 React。

### 6.3 Agent C：React UI

设计方向：

- 主题：深石墨色 Windows 工具台，不做霓虹黑客风。
- 主色：Slate #10151B、Panel #151C24、Line #2A3542、Text #D9E2EC、Accent #63B3A6、Warning #E6B566。
- 字体：界面使用 Segoe UI Variable，终端使用 Cascadia Mono。
- 签名元素：活动窗格左上角显示精细的方向十字标记，同时承担焦点状态和快捷键提示。
- 布局：顶部标签轨道、中间全尺寸 pane canvas、底部紧凑状态栏。

交付：

- 递归 LayoutNode 渲染。
- 每个 Pane 独立 xterm 实例和 FitAddon。
- Tauri Channel 订阅、输入、resize 和清理。
- Ctrl+B Prefix 状态机。
- 标签栏、窗格工具条、状态栏和错误提示。
- 浏览器预览 fallback，真实 Tauri 环境使用 Rust 快照。
- 响应式布局、可见焦点和 reduced-motion。

约束：

- 不在前端生成真实 Pane ID。
- 所有 invoke 使用统一 service 封装和 try/catch。
- 组件卸载必须释放 xterm、ResizeObserver 和 Channel 相关资源。

## 7. 集成顺序

1. 主 Agent 创建脚手架、锁文件和共享错误/IPC 类型。
2. 三个 Agent 并行实现独立目录。
3. 主 Agent 检查共享字段和命名一致性。
4. 在 Tauri lib.rs 注册所有 Commands 和 AppState。
5. 将 Layout Domain 删除结果连接到 TerminalManager 终止操作。
6. 将 React service 映射到完整 Command 列表。
7. 运行格式化、单测、类型检查和构建。
8. 启动真实 Tauri 窗口，验证 PowerShell 输入输出。
9. 对截图进行布局和可用性复核。
10. 更新 docs/ARCHITECTURE.md、docs/IPC.md 和 docs/TESTING.md。

## 8. 验证门槛

### 8.1 Rust

- cargo fmt --all -- --check。
- cargo test。
- cargo clippy --all-targets --all-features -- -D warnings。
- Domain 关键操作均有单元测试。
- 无生产路径 unwrap。

### 8.2 前端

- npm run format:check。
- npm run lint。
- npm run test -- --run。
- npm run build。
- TypeScript strict 无错误。

### 8.3 Tauri

- npm run tauri build -- --debug --no-bundle 或等价编译检查。
- npm run tauri dev 打开独立窗口。
- PowerShell 能接收输入并输出。
- 四向分割均创建真实独立 PTY。
- 四向移动、resize、zoom 和 close 行为符合 PRD。
- 关闭 Pane 后没有继续输出到已销毁前端实例。

## 9. 风险控制

| 风险 | 预防与验证 |
|---|---|
| portable-pty Windows 资源未释放 | close/restart 测试，显式 kill 和移除 handle |
| PTY reader 阻塞退出 | reader 独立线程，关闭 master/writer 后允许 EOF |
| 高频输出压垮 IPC | 4 KiB 批量读取，Channel 有序发送，后续增加背压指标 |
| 前端重复启动同一 Pane | Rust start_terminal 幂等检查并返回稳定错误 |
| React StrictMode 重复副作用 | TerminalPane 使用启动状态和完整 cleanup |
| 布局与终端生命周期脱节 | close 命令对比变更前后 Pane ID 集合后统一回收 |
| 快捷键吞掉终端输入 | 只在 Prefix 激活时拦截第二键，Escape 可取消 |
| 浏览器预览掩盖 Tauri 错误 | 最终验收必须使用真实 Tauri 桌面窗口 |

## 10. 本轮完成定义

- Tauri 工程可以安装依赖、编译并直接打开桌面窗口。
- Rust 和 TypeScript 构建、Lint、格式化检查通过。
- 布局领域测试通过。
- 至少一个 PowerShell 窗格通过真实 PTY 完成输入输出。
- 四向分屏创建独立终端。
- 四向焦点、resize、zoom、close 和标签页可用。
- 核心失败路径显示结构化错误。
- Tauri capabilities 不包含任意 shell 或文件系统权限。
- 所有新增文档位于 docs 目录。
- 未完成的 P1/P2 功能明确记录，不冒充已交付。
