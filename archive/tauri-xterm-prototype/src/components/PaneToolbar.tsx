import type { Direction, PaneSnapshot } from "../types/workspace";

interface PaneToolbarProps {
  readonly pane: PaneSnapshot;
  readonly isActive: boolean;
  readonly isZoomed: boolean;
  readonly runtimeStatus: PaneSnapshot["status"];
  readonly runtimeMessage?: string | null;
  readonly onSplit: (direction: Direction) => void;
  readonly onClose: () => void;
  readonly onToggleZoom: () => void;
  readonly onRestart: () => void;
}

const STATUS_LABELS: Readonly<Record<PaneSnapshot["status"], string>> = {
  starting: "正在启动",
  running: "运行中",
  exited: "已退出",
  error: "错误",
};

const SPLIT_ACTIONS: readonly {
  readonly direction: Direction;
  readonly label: string;
  readonly glyph: string;
}[] = [
  { direction: "left", label: "向左分割", glyph: "←" },
  { direction: "up", label: "向上分割", glyph: "↑" },
  { direction: "down", label: "向下分割", glyph: "↓" },
  { direction: "right", label: "向右分割", glyph: "→" },
];

export function PaneToolbar({
  pane,
  isActive,
  isZoomed,
  runtimeStatus,
  runtimeMessage,
  onSplit,
  onClose,
  onToggleZoom,
  onRestart,
}: PaneToolbarProps): React.JSX.Element {
  const statusLabel = runtimeMessage || STATUS_LABELS[runtimeStatus];

  return (
    <header className="pane-toolbar">
      <div className="pane-toolbar__identity" title={`${pane.title} · ${statusLabel}`}>
        <span className={`pane-status pane-status--${runtimeStatus}`} aria-hidden="true" />
        <span className="pane-toolbar__title">{pane.title}</span>
        <span className="pane-toolbar__profile">{pane.profileId}</span>
      </div>

      {isActive ? (
        <div className="pane-toolbar__actions">
          <div className="pane-toolbar__split-actions" role="group" aria-label="分割窗格">
            {SPLIT_ACTIONS.map(({ direction, label, glyph }) => (
              <button
                className="pane-tool-button pane-tool-button--secondary"
                key={direction}
                type="button"
                aria-label={label}
                title={`${label} · Ctrl+B，Shift+方向键`}
                onClick={() => onSplit(direction)}
              >
                {glyph}
              </button>
            ))}
          </div>

          {runtimeStatus === "exited" || runtimeStatus === "error" ? (
            <button
              className="pane-tool-button"
              type="button"
              aria-label="重新启动终端"
              title="重新启动终端"
              onClick={onRestart}
            >
              ↻
            </button>
          ) : null}
          <button
            className="pane-tool-button"
            type="button"
            aria-label={isZoomed ? "恢复窗格布局" : "最大化窗格"}
            title={`${isZoomed ? "恢复窗格布局" : "最大化窗格"} · Ctrl+B，Z`}
            onClick={onToggleZoom}
          >
            {isZoomed ? "↙" : "□"}
          </button>
          <button
            className="pane-tool-button pane-tool-button--danger"
            type="button"
            aria-label="关闭窗格"
            title="关闭窗格 · Ctrl+B，X"
            onClick={onClose}
          >
            ×
          </button>
        </div>
      ) : (
        <span className="pane-toolbar__inactive-label">{STATUS_LABELS[runtimeStatus]}</span>
      )}
    </header>
  );
}
