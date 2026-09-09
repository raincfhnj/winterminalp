# 故障复盘：Windows Terminal 未可靠触发 F16/F17 隐藏桥接键

## 基本信息

| 字段 | 内容 |
|---|---|
| 日期 | 2026-08-30 |
| 发现人 | Codex 实施与验证 |
| 严重程度 | P1-严重 |
| 影响范围 | 旧桥接表中的向下分屏、上一标签页、标签页 0 等 F16/F17 动作 |
| 关联 Issue/PR | 无 |
| 关联提交 | 桥接表最终排除 F16/F17，见 `src/keymap.rs` 的 `MANAGED_BINDINGS` 与单测 |

## 1. 问题描述

### 1.1 问题场景

在本机 Windows Terminal Stable 1.24.11911.0 安装 29 个托管 Action 与隐藏 keybinding 后，使用真实前台 Terminal 窗口调用 `SendInput` 桥接。F13、F14、F15、F18 对应动作能改变原生窗口状态，但 F16/F17 对应动作没有产生预期效果。

### 1.2 具体表现

- `split-down` 的 `SendInput` 返回完整插入，但随后第一次执行 `closePane` 就关闭了专用测试窗口，说明没有创建第二个 pane。
- `previous-tab` 后窗口标题仍停留在 `WINTERMINAL_E2E_B`。
- `activate-tab-0` 后窗口标题仍停留在 `WINTERMINAL_E2E_B`。
- 同一环境下，F15 的 `next-tab` 和 F18 的 `activate-tab-1` 均成功。
- `RegisterHotKey` 探针确认这些 chord 没有被其他桌面组件全局注册。

### 1.3 错误信息

该问题没有 Win32 错误码；`SendInput` 只确认输入事件被插入，不能确认 Windows Terminal 是否匹配并执行 Action。关键行为证据为：

```text
SplitDown: InputDispatch 成功，第一次 closePane 后目标 HWND 消失
PreviousTab: BeforeTitle=WINTERMINAL_E2E_B, AfterTitle=管理员: WINTERMINAL_E2E_B
NextTab:     BeforeTitle=管理员: WINTERMINAL_E2E_A, AfterTitle=管理员: WINTERMINAL_E2E_B
```

## 2. 临时解决方案

### 2.1 方案描述

先通过 `wter uninstall` 删除旧托管项并恢复 Stable `settings.json` 的原始 SHA-256，再调整桥接表并重新安装。

### 2.2 止血效果

卸载后设置文件恢复到原始哈希 `974431b24fc177d833749904152f7caf6939f6091237f4147bfba82d43183a1d`，无托管项残留；新桥接表安装后向下分屏通过两次关闭行为验证。

### 2.3 临时方案的局限

单纯重试或延长等待时间不能解决问题；曾等待 2 秒后复测，F16 的向下分屏仍未生效。

## 3. 根本原因分析

### 3.1 问题分析过程

1. 首先确认 action fragment 和 Stable `keybindings` 中的 ID、方向及 chord 均正确，`doctor` 报告 29/29 且无冲突。
2. 然后确认 `SendInput` 前台身份校验和插入数量成功，排除发送失败与误投窗口。
3. 使用专用单 pane 窗口执行 `split-down`，再连续执行两次 `closePane`；第一次就关闭窗口，定位为 Action 未执行而非结果读取错误。
4. 通过两个固定标题标签页交叉验证：F15 的 `next-tab` 成功，F16 的 `previous-tab` 失败，F17 的 `activate-tab-0` 失败，F18 的 `activate-tab-1` 成功。
5. 使用 `RegisterHotKey` 逐项探测，排除同 chord 被其他进程全局占用。
6. 最终定位为本机 Stable 1.24 对合成 F16/F17 桥接事件的运行时兼容性问题。官方配置语法虽然接受 F1–F24，但语法有效不等于当前桌面输入链路能可靠派发。

### 3.2 直接原因

旧桥接表连续使用 F13–F24，将多个 P0 动作分配给了实测不可靠的 F16/F17。

**相关代码位置**：`src/keymap.rs` 的 `MANAGED_BINDINGS`；`src/platform/windows/input.rs` 的 `send_bridge_chord`。

**修改前**：

```rust
bridge_chord: chord(true, true, true, 16) // SplitDown
bridge_chord: chord(true, false, true, 16) // PreviousTab
bridge_chord: chord(true, false, true, 17) // ActivateTab0
```

### 3.3 根本原因

- **设计层面**：桥接键选择只依据 Windows Terminal 文档允许的键名范围，没有增加真实输入派发兼容性门槛。
- **开发层面**：初版测试验证了 JSON、唯一性和 `SendInput` 插入数量，但没有验证 Terminal 原生状态变化。
- **流程层面**：真实 Windows 验证安排在集成末尾，因此兼容性问题直到完整安装后才暴露。

### 3.4 为什么没有提前发现

- 单元测试无法模拟 Windows Terminal 的最终快捷键匹配层。
- `SendInput` 成功返回容易被误解为 Action 成功；Win32 API 不提供动作执行回执。
- Windows Terminal 没有公开 pane tree 查询 API，必须设计行为型探针验证。

## 4. 解决方案

### 4.1 根本解决方案

桥接表只使用已在本机真实派发成功的 F13、F14、F15、F18–F24，并通过三组修饰键生成 29 个唯一 chord；F16/F17 完全退出托管表。

**修改文件**：`src/keymap.rs`、`tests/live_bridge.rs`。

**修改后**：

```rust
let reliable_function_keys = [13, 14, 15, 18, 19, 20, 21, 22, 23, 24];
assert!(managed_bindings().iter().all(|binding| {
    reliable_function_keys.contains(&binding.bridge_chord.function_key)
}));
```

同时保留三项运行时证据：

- 四向分屏后第一次关闭 pane 时目标 HWND 保持，第二次关闭时目标 HWND 消失。
- 四向聚焦使用不同 Shell 标题验证活动 pane。
- 上一/下一/索引标签页通过固定标签标题验证。

### 4.2 影响范围评估

- 用户可见 Prefix 键位不变。
- Action ID 和动作语义不变，仅内部隐藏 chord 变化。
- 已安装旧版本必须先卸载再安装；当前开发机已完成该迁移。
- 仍需在其他 Windows/Terminal 版本验证高功能键输入链路。

## 5. 预防措施

### 5.1 代码层面

- [x] 使用单元测试禁止 F16/F17 重新进入托管桥接表。
- [x] 保持 Action、ID、chord 全局唯一性测试。
- [x] 派发前继续校验 HWND、PID、进程启动时间和 Terminal 通道。

### 5.2 测试层面

- [x] 增加默认忽略的真实前台桥接探针 `tests/live_bridge.rs`。
- [x] 使用可观测窗口标题和 HWND 生命周期验证动作结果，而不是只看 `SendInput` 返回值。
- [ ] 在发布矩阵中补充 Stable、Preview 及不同 Windows 版本验证。

### 5.3 监控层面

- [ ] 后续将工作线程的连续派发失败计数暴露给 `doctor` 或本地诊断日志。

### 5.4 流程/规范层面

- [x] 在测试文档中区分“输入已插入”和“Terminal Action 已执行”。
- [ ] 每次修改隐藏 chord 表后必须重新执行至少一个 pane 与一个 tab 的真实行为探针。

## 6. 经验总结（一句话）

> 配置语法允许某个快捷键，不代表合成输入链路能可靠派发；Action Bridge 必须用 Terminal 原生状态变化做真实验收。
