export type Direction = "left" | "right" | "up" | "down";

export type SplitAxis = "row" | "column";

export type PaneStatus = "starting" | "running" | "exited" | "error";

export interface PaneSnapshot {
  readonly id: string;
  readonly title: string;
  readonly profileId: string;
  readonly status: PaneStatus;
  readonly statusMessage?: string | null;
}

export interface PaneLayoutNode {
  readonly kind: "pane";
  readonly pane: PaneSnapshot;
}

export interface SplitLayoutNode {
  readonly kind: "split";
  readonly axis: SplitAxis;
  readonly ratio: number;
  readonly first: LayoutNode;
  readonly second: LayoutNode;
}

export type LayoutNode = PaneLayoutNode | SplitLayoutNode;

export interface TabSnapshot {
  readonly id: string;
  readonly title: string;
  readonly activePaneId: string;
  readonly zoomedPaneId: string | null;
  readonly root: LayoutNode;
}

export interface SessionSnapshot {
  readonly id: string;
  readonly name: string;
  readonly activeTabId: string;
  readonly tabs: readonly TabSnapshot[];
}

export interface AppSnapshot {
  readonly schemaVersion: 1;
  readonly revision: number;
  readonly activeSessionId: string;
  readonly session: SessionSnapshot;
}

export interface TerminalEvent {
  readonly paneId: string;
  readonly sequence: number;
  readonly kind: "output" | "exited" | "error";
  readonly data?: string;
  readonly exitCode?: number;
}

export interface TerminalStarted {
  readonly paneId: string;
  readonly profileId: string;
  readonly processId?: number | null;
  readonly attached: boolean;
}

export interface TerminalDimensions {
  readonly rows: number;
  readonly cols: number;
}

export interface AppError {
  readonly code: string;
  readonly message: string;
  readonly retryable: boolean;
  readonly paneId?: string;
  readonly detail?: string;
}
