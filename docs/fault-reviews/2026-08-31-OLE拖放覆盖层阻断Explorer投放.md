# 故障复盘：OLE 拖放覆盖层阻断 Explorer 向 Windows Terminal 投放文件

## 基本信息

| 字段 | 内容 |
|------|------|
| 日期 | 2026-08-31 |
| 发现人 | 用户手工验收 |
| 严重程度 | P1-严重 |
| 影响范围 | 启用 WinTerminal++ 控制器后，Explorer 向 Windows Terminal PowerShell 窗格的文件/目录拖放 |
| 关联 Issue/PR | 无 |
| 关联提交 | 未提交；工作区为未建立基线的 Git 工作树 |

## 1. 问题描述

### 1.1 问题场景

为把外部文件投放转换为 PowerShell 单引号字面量，代码曾创建一个覆盖整个 Windows Terminal 窗口的、不可激活且低透明度的 OLE `IDropTarget`。用户从 Explorer 拖动含空格、中文或单引号的文件进入 PowerShell 窗格时触发该覆盖层。

### 1.2 具体表现

鼠标进入 Terminal 后显示禁止符号，无法释放文件，也没有路径进入 Shell。这阻断了 Windows Terminal 原有的文件拖放能力。

### 1.3 错误信息

用户报告：

```text
拖动文件到 powershell 的时候鼠标就变成了禁止符号，导致无法成功将文件路径粘贴到 shell 中。
```

无控制器崩溃、无 `doctor` 异常；问题发生在 OLE 目标协商阶段，因此编译、单元测试和桥接健康检查均不能直接发现。

## 2. 临时解决方案

### 2.1 方案描述

立即撤销自定义 OLE 覆盖窗口、`CF_HDROP` 解析、文本注入和 schema 3 的 `[file_drop]` 配置。控制器恢复为只在已命中的原生 pane 分隔线捕获拖动，Explorer 开始的鼠标序列全部交回 Windows Terminal。

### 2.2 止血效果

移除覆盖窗口后，控制器不再覆盖 Windows Terminal 的命中区域，不会自行返回 `DROPEFFECT_NONE`。Windows Terminal 的原生拖放重新成为唯一处理者。

### 2.3 临时方案的局限

这不是对任意 Shell 的“安全转义器”实现。Windows Terminal 的原生投放格式和后续 Shell 解释由 Terminal/Shell 决定；外部控制器不能在不接管其 OLE 目标的前提下改变该数据。

## 3. 根本原因分析

### 3.1 问题分析过程

1. 单元测试验证了 PowerShell 字符串转义和 OLE 数据读取，但用户物理拖放仍显示禁止符号。
2. 该表现说明失败发生在释放前的 drag-over 协商，而不是 `SendInput`、UI Automation 或 Shell 执行阶段。
3. 回查实现发现，覆盖窗口用 `WS_POPUP` 创建，并在指针进入 Terminal 时以 `SetWindowPos(... HWND_TOPMOST ...)` 覆盖完整窗口矩形；alpha=1 仅令它视觉不可见，不会让它在命中测试或 OLE 路由中透明。
4. 覆盖层拥有独立的 `RegisterDragDrop` 目标，却不能可靠替代 Windows Terminal 已注册的内部目标和其拖放协商；当它未接受有效数据时，Explorer 得到 `DROPEFFECT_NONE`，即禁止符号。
5. 最终确定：在外部进程创建覆盖 `IDropTarget` 是错误的所有权边界。必须删除，而不是继续调整透明度、计时或 `DROPEFFECT`。

### 3.2 直接原因

已删除的 `src/controller/file_drop.rs` 通过 `CreateWindowExW(WS_POPUP)`、`RegisterDragDrop` 和 `SetWindowPos(HWND_TOPMOST)` 将自有窗口放在 Terminal 上方。该窗口视觉透明但仍接收拖放命中，直接遮蔽 Terminal 的原生 `IDropTarget`。

当前安全相关代码位置：

- `src/controller.rs:159-199`：只在 `PointerDragState` 已捕获分隔线时消费 mouse down/move/up。
- `src/controller/pointer.rs:49-142`：外部拖动从分隔线外开始时永远返回 `Pass`。
- `src/platform/windows/hook.rs:417-447`：未消费的低级鼠标事件继续调用 `CallNextHookEx`。

### 3.3 根本原因

- **设计层面**：错误假设外部进程可通过覆盖窗口安全地过滤另一个进程的 OLE 投放。OLE drop target 以实际命中窗口为边界，覆盖层必然改变路由与原生窗口行为。
- **开发层面**：把“路径安全转义”与“必须接管拖放数据”混为一谈，未在设计阶段将 Windows Terminal 的 UI/OLE 所有权列为不可跨越边界。
- **流程层面**：物理 Explorer → Terminal 投放被列为手工验收项，但在发布前没有完成；单元测试只覆盖纯转义函数，不能证明 OLE 命中路由正确。

### 3.4 为什么没有提前发现

- 代码审查阶段未检查“视觉透明窗口仍参与 hit-test/OLE 路由”的 Win32 语义。
- 测试阶段没有真实 Explorer 拖放验收；合成鼠标输入会被 Hook 透传，不能替代物理 OLE 拖放。
- `doctor` 只验证 Terminal action bridge 与配置完整性，不观察 OLE drag-over effect，因而保持 healthy 是预期现象。

## 4. 解决方案

### 4.1 根本解决方案

删除 `src/controller/file_drop.rs`，并撤销其对 `controller.rs`、`controller/action_worker.rs`、`platform/windows/input.rs`、`platform/windows/foreground.rs`、`config.rs` 和 Windows API feature 的依赖。配置 schema 恢复为 2。

修改后，控制器对未在原生分隔线开始的拖动返回 `HookDecision::Pass`，不创建窗口、不注册 OLE target、不注入路径文本。文件投放由 Windows Terminal 原生处理。

### 4.2 影响范围评估

- 消除了 Explorer 拖放被禁止的回归。
- 保留已验证的 pane 分隔线 resize reducer 和 action worker 批处理。
- 不再提供控制器侧 PowerShell 路径转义；这是为恢复正确的 UI/OLE 所有权和可用性作出的明确边界。

## 5. 预防措施

### 5.1 代码层面

- [x] 禁止控制器为 Windows Terminal 创建覆盖 OLE `IDropTarget` 或子类化其窗口。
- [x] 在架构文档中明确 Windows Terminal 拥有 drag-and-drop UI 与 Shell 输入契约。
- [ ] 若未来需要变换 dropped payload，必须由 Windows Terminal 提供扩展点，或由用户明确选择一个独立、可见且自有的投放窗口；不得覆盖 Terminal。

### 5.2 测试层面

- [x] 保留 `PointerDragState` 的“外部拖动永不捕获”单元测试。
- [ ] 所有涉及原生鼠标或 OLE 的改动，release 前必须在专用 Terminal 窗口完成 Explorer 物理拖放验收，观察允许/禁止光标和释放结果。

### 5.3 监控层面

- [ ] 若以后有受控的文件投放功能，记录仅包含计数和分类的接受/拒绝结果，不记录文件路径；`doctor` 不得被当作拖放可用性证明。

### 5.4 流程/规范层面

- [x] 将“合成鼠标与单元测试不能证明 OLE 拖放”保留在测试文档。
- [ ] 设计评审增加“目标窗口/消息/OLE 所有权”检查项，先确认 API 扩展点再讨论覆盖层方案。

## 6. 经验总结（一句话）

> 对另一个进程的原生拖放，视觉透明覆盖窗口仍会改变命中和 OLE 路由；没有官方扩展点时，应保持透传而不是试图在外部改写 payload。
