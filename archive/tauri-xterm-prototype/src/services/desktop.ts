import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import {
  nearestPaneId,
  paneIdsInLayout,
  removePaneFromLayout,
  resizeLayout,
  splitLayout,
} from "../domain/layout";
import type {
  AppError,
  AppSnapshot,
  Direction,
  PaneSnapshot,
  TabSnapshot,
  TerminalDimensions,
  TerminalEvent,
  TerminalStarted,
} from "../types/workspace";

const RESIZE_AMOUNT = 0.05;
const PREVIEW_PANE_IDS = [
  "preview-pane-main",
  "preview-pane-build",
  "preview-pane-tests",
  "preview-pane-logs",
  "preview-pane-shell",
  "preview-pane-server",
] as const;
const PREVIEW_TAB_IDS = [
  "preview-tab-workspace",
  "preview-tab-notes",
  "preview-tab-extra",
] as const;

export interface TerminalSubscription {
  readonly started: TerminalStarted;
  dispose(): void;
}

export interface DesktopService {
  readonly isDesktop: boolean;
  getAppSnapshot(): Promise<AppSnapshot>;
  splitPane(direction: Direction): Promise<AppSnapshot>;
  focusPane(direction: Direction): Promise<AppSnapshot>;
  resizePane(direction: Direction, amount?: number): Promise<AppSnapshot>;
  closePane(): Promise<AppSnapshot>;
  toggleZoom(): Promise<AppSnapshot>;
  createTab(): Promise<AppSnapshot>;
  activateTab(tabId: string): Promise<AppSnapshot>;
  closeTab(tabId: string): Promise<AppSnapshot>;
  startTerminal(
    paneId: string,
    dimensions: TerminalDimensions,
    onEvent: (event: TerminalEvent) => void,
  ): Promise<TerminalSubscription>;
  writeTerminal(paneId: string, data: string): Promise<void>;
  resizeTerminal(paneId: string, dimensions: TerminalDimensions): Promise<void>;
  restartTerminal(
    paneId: string,
    dimensions: TerminalDimensions,
    onEvent: (event: TerminalEvent) => void,
  ): Promise<TerminalSubscription>;
}

export class DesktopServiceError extends Error {
  readonly appError: AppError;

  constructor(appError: AppError) {
    super(appError.message);
    this.name = "DesktopServiceError";
    this.appError = appError;
  }
}

class WinTerminalDesktopService implements DesktopService {
  readonly isDesktop = isTauri();
  readonly #preview = new BrowserPreview();

  async getAppSnapshot(): Promise<AppSnapshot> {
    return this.isDesktop
      ? invokeCommand<AppSnapshot>("get_app_snapshot")
      : this.#preview.getAppSnapshot();
  }

  async splitPane(direction: Direction): Promise<AppSnapshot> {
    return this.isDesktop
      ? invokeCommand<AppSnapshot>("split_pane", { direction })
      : this.#preview.splitPane(direction);
  }

  async focusPane(direction: Direction): Promise<AppSnapshot> {
    return this.isDesktop
      ? invokeCommand<AppSnapshot>("focus_pane", { direction })
      : this.#preview.focusPane(direction);
  }

  async resizePane(direction: Direction, amount = RESIZE_AMOUNT): Promise<AppSnapshot> {
    return this.isDesktop
      ? invokeCommand<AppSnapshot>("resize_pane", { direction, amount })
      : this.#preview.resizePane(direction, amount);
  }

  async closePane(): Promise<AppSnapshot> {
    return this.isDesktop ? invokeCommand<AppSnapshot>("close_pane") : this.#preview.closePane();
  }

  async toggleZoom(): Promise<AppSnapshot> {
    return this.isDesktop ? invokeCommand<AppSnapshot>("toggle_zoom") : this.#preview.toggleZoom();
  }

  async createTab(): Promise<AppSnapshot> {
    return this.isDesktop ? invokeCommand<AppSnapshot>("create_tab") : this.#preview.createTab();
  }

  async activateTab(tabId: string): Promise<AppSnapshot> {
    return this.isDesktop
      ? invokeCommand<AppSnapshot>("activate_tab", { tabId })
      : this.#preview.activateTab(tabId);
  }

  async closeTab(tabId: string): Promise<AppSnapshot> {
    return this.isDesktop
      ? invokeCommand<AppSnapshot>("close_tab", { tabId })
      : this.#preview.closeTab(tabId);
  }

  async startTerminal(
    paneId: string,
    dimensions: TerminalDimensions,
    onEvent: (event: TerminalEvent) => void,
  ): Promise<TerminalSubscription> {
    if (!this.isDesktop) {
      return this.#preview.startTerminal(paneId, onEvent);
    }

    return subscribeToTerminal("start_terminal", paneId, dimensions, onEvent);
  }

  async writeTerminal(paneId: string, data: string): Promise<void> {
    if (!this.isDesktop) {
      return this.#preview.writeTerminal(paneId, data);
    }

    await invokeCommand<void>("write_terminal", { paneId, data });
  }

  async resizeTerminal(paneId: string, dimensions: TerminalDimensions): Promise<void> {
    if (!this.isDesktop) {
      return;
    }

    await invokeCommand<void>("resize_terminal", { paneId, ...dimensions });
  }

  async restartTerminal(
    paneId: string,
    dimensions: TerminalDimensions,
    onEvent: (event: TerminalEvent) => void,
  ): Promise<TerminalSubscription> {
    if (!this.isDesktop) {
      return this.#preview.restartTerminal(paneId, onEvent);
    }

    return subscribeToTerminal("restart_terminal", paneId, dimensions, onEvent);
  }
}

async function subscribeToTerminal(
  command: "start_terminal" | "restart_terminal",
  paneId: string,
  dimensions: TerminalDimensions,
  onEvent: (event: TerminalEvent) => void,
): Promise<TerminalSubscription> {
  let isDisposed = false;
  const channel = new Channel<TerminalEvent>((event) => {
    if (!isDisposed) {
      onEvent(event);
    }
  });
  let started: TerminalStarted;
  try {
    started = await invokeCommand<TerminalStarted>(command, {
      paneId,
      ...dimensions,
      channel,
    });
  } catch (error: unknown) {
    isDisposed = true;
    channel.onmessage = () => undefined;
    throw error;
  }

  return {
    started,
    dispose() {
      isDisposed = true;
      channel.onmessage = () => undefined;
    },
  };
}

async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    throw new DesktopServiceError(normalizeAppError(command, error));
  }
}

function normalizeAppError(command: string, error: unknown): AppError {
  if (isAppError(error)) {
    return error;
  }

  const message = error instanceof Error ? error.message : String(error);
  return {
    code: "IPC_COMMAND_FAILED",
    message: `无法执行 ${command}`,
    detail: message,
    retryable: true,
  };
}

function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const candidate = value as Partial<AppError>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.message === "string" &&
    typeof candidate.retryable === "boolean"
  );
}

class BrowserPreview {
  #snapshot: AppSnapshot = createPreviewSnapshot();
  #nextPaneIndex = 3;
  #nextTabIndex = 1;
  readonly #terminalSinks = new Map<string, (event: TerminalEvent) => void>();
  readonly #terminalSequences = new Map<string, number>();

  async getAppSnapshot(): Promise<AppSnapshot> {
    return this.#snapshot;
  }

  async splitPane(direction: Direction): Promise<AppSnapshot> {
    const tab = activeTab(this.#snapshot);
    const paneId = this.#nextAvailablePaneId();
    if (!paneId) {
      return this.#snapshot;
    }

    const newPane = previewPane(paneId, "PowerShell");
    return this.#replaceActiveTab({
      ...tab,
      activePaneId: paneId,
      zoomedPaneId: null,
      root: splitLayout(tab.root, tab.activePaneId, newPane, direction),
    });
  }

  async focusPane(direction: Direction): Promise<AppSnapshot> {
    const tab = activeTab(this.#snapshot);
    const paneId = nearestPaneId(tab.root, tab.activePaneId, direction);
    return paneId ? this.#replaceActiveTab({ ...tab, activePaneId: paneId }) : this.#snapshot;
  }

  async resizePane(direction: Direction, amount: number): Promise<AppSnapshot> {
    const tab = activeTab(this.#snapshot);
    return this.#replaceActiveTab({
      ...tab,
      root: resizeLayout(tab.root, tab.activePaneId, direction, amount),
    });
  }

  async closePane(): Promise<AppSnapshot> {
    const tab = activeTab(this.#snapshot);
    const remainingRoot = removePaneFromLayout(tab.root, tab.activePaneId);

    if (!remainingRoot) {
      return this.#replaceActiveTab(createPreviewTab(tab.id, tab.title, "preview-pane-main"));
    }

    const nextPaneId = paneIdsInLayout(remainingRoot)[0];
    return this.#replaceActiveTab({
      ...tab,
      activePaneId: nextPaneId,
      zoomedPaneId: null,
      root: remainingRoot,
    });
  }

  async toggleZoom(): Promise<AppSnapshot> {
    const tab = activeTab(this.#snapshot);
    return this.#replaceActiveTab({
      ...tab,
      zoomedPaneId: tab.zoomedPaneId ? null : tab.activePaneId,
    });
  }

  async createTab(): Promise<AppSnapshot> {
    const tabId = this.#nextAvailableTabId();
    const paneId = this.#nextAvailablePaneId();
    if (!tabId || !paneId) {
      return this.#snapshot;
    }

    const tab = createPreviewTab(
      tabId,
      `terminal ${this.#snapshot.session.tabs.length + 1}`,
      paneId,
    );
    this.#snapshot = nextRevision(this.#snapshot, {
      ...this.#snapshot.session,
      activeTabId: tab.id,
      tabs: [...this.#snapshot.session.tabs, tab],
    });
    return this.#snapshot;
  }

  async activateTab(tabId: string): Promise<AppSnapshot> {
    if (!this.#snapshot.session.tabs.some((tab) => tab.id === tabId)) {
      return this.#snapshot;
    }

    this.#snapshot = nextRevision(this.#snapshot, {
      ...this.#snapshot.session,
      activeTabId: tabId,
    });
    return this.#snapshot;
  }

  async closeTab(tabId: string): Promise<AppSnapshot> {
    const tabs = this.#snapshot.session.tabs.filter((tab) => tab.id !== tabId);
    const nextTabs =
      tabs.length > 0
        ? tabs
        : [createPreviewTab("preview-tab-workspace", "workspace", "preview-pane-main")];
    const activeTabId = nextTabs.some((tab) => tab.id === this.#snapshot.session.activeTabId)
      ? this.#snapshot.session.activeTabId
      : nextTabs[0].id;
    this.#snapshot = nextRevision(this.#snapshot, {
      ...this.#snapshot.session,
      activeTabId,
      tabs: nextTabs,
    });
    return this.#snapshot;
  }

  async startTerminal(
    paneId: string,
    onEvent: (event: TerminalEvent) => void,
  ): Promise<TerminalSubscription> {
    this.#terminalSinks.set(paneId, onEvent);
    queueMicrotask(() => {
      this.#emit(
        paneId,
        "output",
        `\u001b[38;2;99;179;166mWinTerminal++ preview\u001b[0m\r\n\u001b[38;2;129;145;160mBrowser mode · desktop IPC is not connected\u001b[0m\r\n\r\nPS D:\\workspace> `,
      );
    });

    return {
      started: {
        paneId,
        profileId: "powershell",
        attached: false,
      },
      dispose: () => {
        if (this.#terminalSinks.get(paneId) === onEvent) {
          this.#terminalSinks.delete(paneId);
        }
      },
    };
  }

  async writeTerminal(paneId: string, data: string): Promise<void> {
    const visibleData = data.replace(/\r/g, "\r\nPS D:\\workspace> ");
    this.#emit(paneId, "output", visibleData);
  }

  async restartTerminal(
    paneId: string,
    onEvent: (event: TerminalEvent) => void,
  ): Promise<TerminalSubscription> {
    const subscription = await this.startTerminal(paneId, onEvent);
    queueMicrotask(() => this.#emit(paneId, "output", "\r\nTerminal restarted.\r\n"));
    return subscription;
  }

  #replaceActiveTab(tab: TabSnapshot): AppSnapshot {
    this.#snapshot = nextRevision(this.#snapshot, {
      ...this.#snapshot.session,
      tabs: this.#snapshot.session.tabs.map((item) => (item.id === tab.id ? tab : item)),
    });
    return this.#snapshot;
  }

  #nextAvailablePaneId(): string | null {
    const usedPaneIds = new Set(
      this.#snapshot.session.tabs.flatMap((tab) => paneIdsInLayout(tab.root)),
    );
    for (let offset = 0; offset < PREVIEW_PANE_IDS.length; offset += 1) {
      const index = (this.#nextPaneIndex + offset) % PREVIEW_PANE_IDS.length;
      const paneId = PREVIEW_PANE_IDS[index];
      if (!usedPaneIds.has(paneId)) {
        this.#nextPaneIndex = index + 1;
        return paneId;
      }
    }

    return null;
  }

  #nextAvailableTabId(): string | null {
    const usedTabIds = new Set(this.#snapshot.session.tabs.map((tab) => tab.id));
    for (let offset = 0; offset < PREVIEW_TAB_IDS.length; offset += 1) {
      const index = (this.#nextTabIndex + offset) % PREVIEW_TAB_IDS.length;
      const tabId = PREVIEW_TAB_IDS[index];
      if (!usedTabIds.has(tabId)) {
        this.#nextTabIndex = index + 1;
        return tabId;
      }
    }

    return null;
  }

  #emit(paneId: string, kind: TerminalEvent["kind"], data?: string): void {
    const sink = this.#terminalSinks.get(paneId);
    if (!sink) {
      return;
    }

    const sequence = (this.#terminalSequences.get(paneId) ?? 0) + 1;
    this.#terminalSequences.set(paneId, sequence);
    sink({ paneId, sequence, kind, data });
  }
}

function createPreviewSnapshot(): AppSnapshot {
  const main = previewPane("preview-pane-main", "PowerShell");
  const build = previewPane("preview-pane-build", "build · npm");
  const tests = previewPane("preview-pane-tests", "tests · vitest");

  return {
    schemaVersion: 1,
    revision: 1,
    activeSessionId: "preview-session",
    session: {
      id: "preview-session",
      name: "local workspace",
      activeTabId: "preview-tab-workspace",
      tabs: [
        {
          id: "preview-tab-workspace",
          title: "workspace",
          activePaneId: main.id,
          zoomedPaneId: null,
          root: {
            kind: "split",
            axis: "row",
            ratio: 0.58,
            first: { kind: "pane", pane: main },
            second: {
              kind: "split",
              axis: "column",
              ratio: 0.52,
              first: { kind: "pane", pane: build },
              second: { kind: "pane", pane: tests },
            },
          },
        },
      ],
    },
  };
}

function createPreviewTab(id: string, title: string, paneId: string): TabSnapshot {
  return {
    id,
    title,
    activePaneId: paneId,
    zoomedPaneId: null,
    root: { kind: "pane", pane: previewPane(paneId, "PowerShell") },
  };
}

function previewPane(id: string, title: string): PaneSnapshot {
  return {
    id,
    title,
    profileId: "powershell",
    status: "running",
  };
}

function activeTab(snapshot: AppSnapshot): TabSnapshot {
  return (
    snapshot.session.tabs.find((tab) => tab.id === snapshot.session.activeTabId) ??
    snapshot.session.tabs[0]
  );
}

function nextRevision(snapshot: AppSnapshot, session: AppSnapshot["session"]): AppSnapshot {
  return {
    ...snapshot,
    revision: snapshot.revision + 1,
    session,
  };
}

export const desktopService: DesktopService = new WinTerminalDesktopService();
