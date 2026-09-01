import type { AppSnapshot, PaneSnapshot } from "../types/workspace";

interface StatusBarProps {
  readonly snapshot: AppSnapshot;
  readonly activePane: PaneSnapshot;
  readonly isPrefixActive: boolean;
  readonly isDesktop: boolean;
  readonly isBusy: boolean;
}

const STATUS_LABELS: Readonly<Record<PaneSnapshot["status"], string>> = {
  starting: "starting",
  running: "running",
  exited: "exited",
  error: "error",
};

export function StatusBar({
  snapshot,
  activePane,
  isPrefixActive,
  isDesktop,
  isBusy,
}: StatusBarProps): React.JSX.Element {
  return (
    <footer className={`status-bar${isPrefixActive ? " status-bar--prefix" : ""}`}>
      <div className="status-bar__cluster">
        <span className={`connection-dot${isDesktop ? " connection-dot--desktop" : ""}`} />
        <span>{isDesktop ? "LOCAL" : "PREVIEW"}</span>
        <span className="status-bar__divider" aria-hidden="true" />
        <span className="status-bar__session">{snapshot.session.name}</span>
      </div>

      <div className="status-bar__prefix" role="status" aria-live="polite">
        {isPrefixActive ? (
          <>
            <kbd>Ctrl+B</kbd>
            <span>Prefix · 选择方向或命令</span>
          </>
        ) : isBusy ? (
          <span>正在同步 Rust 状态…</span>
        ) : (
          <>
            <kbd>Ctrl+B</kbd>
            <span>进入 Prefix</span>
          </>
        )}
      </div>

      <div className="status-bar__cluster status-bar__cluster--right">
        <span>{activePane.profileId}</span>
        <span className={`status-text status-text--${activePane.status}`}>
          {STATUS_LABELS[activePane.status]}
        </span>
        <span>r{snapshot.revision}</span>
      </div>
    </footer>
  );
}
