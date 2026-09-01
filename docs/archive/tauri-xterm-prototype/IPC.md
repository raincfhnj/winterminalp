# WinTerminal++ Tauri IPC 契约

本文档定义 React WebView 和 Tauri Rust Core 之间的 P0 契约。字段以 camelCase 在 IPC 中传输，Rust 内部使用 snake_case。

## 1. 通用规则

- 所有控制操作通过已注册的 Tauri Command。
- 所有 Command 返回 Promise 语义的 Result。
- 高频终端输出通过 Channel 传输。
- 前端不得直接 emit 布局状态事件。
- 每个成功的领域变更返回完整 AppSnapshot。
- revision 是 Rust 状态版本，不是数据库版本。

## 2. 公共类型

### Direction

    left | right | up | down

### Axis

    row | column

### PaneStatus

    starting | running | exited | error

### AppSnapshot

    interface AppSnapshot {
      schemaVersion: 1;
      revision: number;
      activeSessionId: string;
      session: SessionSnapshot;
    }

### SessionSnapshot

    interface SessionSnapshot {
      id: string;
      name: string;
      activeTabId: string;
      tabs: TabSnapshot[];
    }

### TabSnapshot

    interface TabSnapshot {
      id: string;
      title: string;
      activePaneId: string;
      zoomedPaneId: string | null;
      root: LayoutNode;
    }

### LayoutNode

    type LayoutNode =
      | {
          kind: "pane";
          pane: PaneSnapshot;
        }
      | {
          kind: "split";
          axis: Axis;
          ratio: number;
          first: LayoutNode;
          second: LayoutNode;
        };

### PaneSnapshot

    interface PaneSnapshot {
      id: string;
      title: string;
      profileId: string;
      status: PaneStatus;
      statusMessage?: string | null;
    }

### AppError

    interface AppError {
      code: string;
      message: string;
      retryable: boolean;
      paneId?: string;
      detail?: string;
    }

### TerminalEvent

    interface TerminalEvent {
      paneId: string;
      sequence: number;
      kind: "output" | "exited" | "error";
      data?: string;
      exitCode?: number;
    }

### TerminalStarted

    interface TerminalStarted {
      paneId: string;
      profileId: string;
      attached: boolean;
    }

## 3. 布局 Commands

### get_app_snapshot

参数：无。

返回：AppSnapshot。

### split_pane

参数：

    {
      direction: Direction
    }

返回：AppSnapshot。

规则：

- 目标始终是当前标签页的活动 Pane。
- 成功后新 Pane 成为活动 Pane。
- 新 Pane 由 Rust 生成 ID。

### focus_pane

参数：

    {
      direction: Direction
    }

返回：AppSnapshot。

没有候选 Pane 时返回未修改快照，不增加 revision。

### resize_pane

参数：

    {
      direction: Direction;
      amount: number;
    }

返回：AppSnapshot。

amount 是归一化比例步长，服务端限制到安全范围。

### close_pane

参数：无。

返回：AppSnapshot。

集成层同时终止 removedPaneIds 对应 PTY。

### toggle_zoom

参数：无。

返回：AppSnapshot。

只修改 zoomedPaneId，不修改 LayoutNode。

## 4. 标签页 Commands

### create_tab

参数：无。

返回：AppSnapshot。

新标签页包含一个 starting Pane 并立即激活。

### activate_tab

参数：

    {
      tabId: string
    }

返回：AppSnapshot。

### close_tab

参数：

    {
      tabId: string
    }

返回：AppSnapshot。

关闭最后一个标签页时，Rust 自动创建新的默认标签页。

## 5. 终端 Commands

### start_terminal

参数：

    {
      paneId: string;
      profileId?: string;
      cwd?: string;
      rows: number;
      cols: number;
      channel: Channel<TerminalEvent>;
    }

返回：TerminalStarted。

规则：

- paneId 必须存在于 AppModel。
- profileId 必须是 Rust 内置 Profile。
- rows 和 cols 必须在服务端范围内。
- Pane 已运行时只替换 Channel sink，attached 返回 true。
- 新启动时 attached 返回 false。

### write_terminal

参数：

    {
      paneId: string;
      data: string;
    }

返回：void。

输入按 UTF-8 字节写入 PTY。前端可以做短时间批处理，但不能重排输入。

### resize_terminal

参数：

    {
      paneId: string;
      rows: number;
      cols: number;
    }

返回：void。

重复尺寸允许成功，便于 ResizeObserver 去抖后的幂等调用。

### restart_terminal

参数：

    {
      paneId: string;
      rows: number;
      cols: number;
      channel: Channel<TerminalEvent>;
    }

返回：TerminalStarted。

已有进程仍运行时拒绝 restart。已退出时复用旧 Profile/cwd；首次启动失败、尚无 handle 时使用 Pane 的 Profile 和默认 cwd 重试。

## 6. 终端事件

### output

    {
      paneId,
      sequence,
      kind: "output",
      data
    }

### exited

    {
      paneId,
      sequence,
      kind: "exited",
      exitCode
    }

### error

    {
      paneId,
      sequence,
      kind: "error",
      data
    }

前端规则：

- 忽略 paneId 不匹配当前 TerminalPane 的事件。
- 忽略 sequence 小于或等于已处理 sequence 的重复事件。
- output 写入 xterm。
- exited 保留屏幕内容并展示重启操作。
- error 展示持久状态，不只写入控制台。

## 7. 错误码

| code | 含义 | retryable |
|---|---|---|
| INVALID_DIRECTION | 方向参数无效 | false |
| INVALID_AMOUNT | resize 步长无效 | false |
| PANE_NOT_FOUND | Pane 不存在 | false |
| TAB_NOT_FOUND | Tab 不存在 | false |
| INVALID_PROFILE | Profile 不存在或不可用 | false |
| INVALID_TERMINAL_SIZE | PTY 行列数无效 | false |
| TERMINAL_NOT_RUNNING | 目标终端未运行 | true |
| TERMINAL_ALREADY_RUNNING | restart 目标仍在运行 | false |
| TERMINAL_START_FAILED | PTY 或 Shell 启动失败 | true |
| TERMINAL_IO_FAILED | PTY 读写失败 | true |
| STATE_LOCK_FAILED | Rust 状态锁异常 | true |
| INTERNAL_ERROR | 未分类内部错误 | true |

## 8. Command 注册检查

src-tauri/src/lib.rs 必须在 generate_handler 中注册本文档所有 P0 Commands。新增 Command 时同时更新：

1. Rust handler。
2. generate_handler 注册。
3. TypeScript service。
4. 本文档。
5. Command 测试或端到端用例。

## 9. 兼容性

- P0 契约版本为 1，并由 AppSnapshot.schemaVersion 显式携带。
- 可选字段可以在版本 1 内增加。
- 字段删除、重命名或语义变化需要升级契约版本。
- P1 Daemon 协议与 Tauri IPC 版本分别维护，不复用隐式结构。
