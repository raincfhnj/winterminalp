# WinTerminal++ 测试与验收

## 自动验证

在仓库根目录运行：

    npm run format:check
    npm run lint
    npm run test -- --run
    npm run build
    npm run tauri -- build --debug --no-bundle

在 `src-tauri` 目录运行：

    cargo fmt --all -- --check
    cargo test --lib
    cargo clippy --all-targets --all-features -- -D warnings

依赖审计使用官方 npm registry：

    npm audit --audit-level=high --registry=https://registry.npmjs.org

## 2026-08-30 验证基线

| 检查 | 结果 |
|---|---|
| TypeScript/React 单元测试 | 5 个文件、14 项测试通过 |
| Rust 单元与 Windows PTY 测试 | 30 项测试通过 |
| ESLint、Prettier、rustfmt、Clippy | 通过 |
| Vite production build | 通过，xterm 已拆分为独立 chunk |
| Tauri debug no-bundle build | 通过，生成 `src-tauri/target/debug/winterminal.exe` |
| npm audit | 0 个已知漏洞 |
| 浏览器预览 | 三窗格初始树、第四窗格创建、Prefix 提示、方向焦点移动和响应式工作台可见 |
| 真实 Tauri 启动 | 检测到标题为 `WinTerminal++` 的窗口和 WebView2 |
| 真实 PowerShell 创建 | 检测到由 `winterminal.exe` 创建的 `powershell.exe -NoLogo` 子进程 |
| 退出清理 | 测试启动的应用、PowerShell、WebView2 和 Vite 进程均已停止 |

Rust PTY 测试会在 Windows 上通过 ConPTY 启动真实 CMD，完成光标查询握手、输入输出、重复 attach PID 复用和退出事件验证。PowerShell 已在真实 Tauri 运行中验证创建；终端 UI 的自动键盘注入不纳入自动化基线。

## 人工桌面验收

启动：

    npm run tauri dev

依次检查：

1. 首个 PowerShell 出现提示符且能输入、输出。
2. 使用四个 `Shift+方向键` Prefix 组合创建窗格。
3. 使用方向键 Prefix 组合在不规则布局中移动焦点。
4. 使用 `Ctrl+方向键` Prefix 组合改变最近匹配的分隔比例。
5. 新建、切换和关闭标签页；关闭最后一个标签页后自动出现默认标签页。
6. 最大化与恢复窗格不会改变原布局比例。
7. 关闭窗格后对应 Shell 进程停止，剩余窗格填满空间。
8. Shell 退出后保留输出并可使用工具栏重新启动。

P1 持久会话尚未实现：关闭 WinTerminal++ 会结束其 PTY，不能把当前版本当作可 detach/reattach 的完整 tmux 替代品。
