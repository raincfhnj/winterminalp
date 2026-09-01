# WinTerminal++ 开发指南

WinTerminal++ 使用 Tauri v2、Rust、React、TypeScript 和 Vite。

## 环境要求

- Windows 10/11。
- Node.js 与 npm。
- Rust stable MSVC toolchain。
- Microsoft C++ Build Tools 与 WebView2 开发环境。

## 本地开发

安装依赖：

    npm install

启动浏览器界面：

    npm run dev

启动 Tauri 桌面应用：

    npm run tauri dev

生成可直接运行的本地 debug 可执行文件：

    npm run tauri -- build --debug --no-bundle

输出位置：`src-tauri/target/debug/winterminal.exe`。

## 常用验证

    npm run format:check
    npm run lint
    npm run test -- --run
    npm run build

Rust 验证：

    cd src-tauri
    cargo fmt --all -- --check
    cargo test
    cargo clippy --all-targets --all-features -- -D warnings

详细实施顺序与模块所有权见 docs/IMPLEMENTATION_PLAN.md。

默认操作见 docs/SHORTCUTS.md，完整验证基线和人工验收步骤见 docs/TESTING.md。
