# Tauri/xterm 原型归档

该目录保存 WinTerminal++ v0.1 的产品、架构、IPC、开发、测试和快捷键文档。对应可构建源码位于 `archive/tauri-xterm-prototype/`。

归档原因：产品方向已于 2026-08-30 改为直接复用 Windows Terminal 原生窗口，不再自绘 Tauri/React/xterm 界面。

归档状态：

- 仅供历史参考，不是默认产品入口。
- 原来的源码、测试、lockfile、Tauri capabilities 和 icons 均保留。
- 原型完成时 Rust 测试、Clippy、rustfmt、前端测试、Lint 和构建均通过。
- `node_modules`、`dist` 和 Rust `target` 属于生成物，不纳入 Git 跟踪。
- 仓库没有历史 commit；本次归档未创建 commit 或 tag。

如需运行旧原型，应进入 `archive/tauri-xterm-prototype/` 并遵循本目录中的旧 `DEVELOPMENT.md`。旧原型不得与根目录 v0.2 控制器的验收结果混为一谈。
