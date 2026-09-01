import { useCallback, useEffect, useRef, useState } from "react";
import "./App.css";
import { ErrorNotice } from "./components/ErrorNotice";
import { SplitPaneTree } from "./components/SplitPaneTree";
import { StatusBar } from "./components/StatusBar";
import { TabBar } from "./components/TabBar";
import { findPane, focusPathToPane, paneIdsInLayout } from "./domain/layout";
import { usePrefixShortcuts } from "./hooks/usePrefixShortcuts";
import type { PrefixCommand } from "./keyboard/prefix";
import { DesktopServiceError, desktopService } from "./services/desktop";
import type { AppError, AppSnapshot, Direction, TabSnapshot } from "./types/workspace";

type SnapshotOperation = (snapshot: AppSnapshot) => Promise<AppSnapshot>;

function App(): React.JSX.Element {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isBusy, setIsBusy] = useState(false);
  const snapshotRef = useRef<AppSnapshot | null>(null);
  const commandQueueRef = useRef<Promise<void>>(Promise.resolve());
  const pendingCommandsRef = useRef(0);
  const isMountedRef = useRef(true);

  const acceptSnapshot = useCallback((nextSnapshot: AppSnapshot): void => {
    if (!isMountedRef.current) {
      return;
    }

    const currentRevision = snapshotRef.current?.revision ?? -1;
    if (nextSnapshot.revision < currentRevision) {
      return;
    }

    snapshotRef.current = nextSnapshot;
    setSnapshot(nextSnapshot);
  }, []);

  const reportError = useCallback((value: unknown): void => {
    if (isMountedRef.current) {
      setError(toAppError(value));
    }
  }, []);

  const synchronizeSnapshot = useCallback((): void => {
    void desktopService.getAppSnapshot().then(acceptSnapshot).catch(reportError);
  }, [acceptSnapshot, reportError]);

  const refreshSnapshot = useCallback(async (): Promise<void> => {
    setIsLoading(true);
    try {
      acceptSnapshot(await desktopService.getAppSnapshot());
      setError(null);
    } catch (value: unknown) {
      reportError(value);
    } finally {
      if (isMountedRef.current) {
        setIsLoading(false);
      }
    }
  }, [acceptSnapshot, reportError]);

  useEffect(() => {
    isMountedRef.current = true;
    void desktopService
      .getAppSnapshot()
      .then((nextSnapshot) => {
        acceptSnapshot(nextSnapshot);
        setError(null);
      })
      .catch(reportError)
      .finally(() => {
        if (isMountedRef.current) {
          setIsLoading(false);
        }
      });
    return () => {
      isMountedRef.current = false;
    };
  }, [acceptSnapshot, reportError]);

  const runCommand = useCallback(
    (operation: SnapshotOperation): void => {
      pendingCommandsRef.current += 1;
      setIsBusy(true);
      commandQueueRef.current = commandQueueRef.current.then(async () => {
        try {
          const currentSnapshot = snapshotRef.current;
          if (!currentSnapshot) {
            return;
          }

          const nextSnapshot = await operation(currentSnapshot);
          acceptSnapshot(nextSnapshot);
          if (isMountedRef.current) {
            setError(null);
          }
        } catch (value: unknown) {
          reportError(value);
        } finally {
          pendingCommandsRef.current -= 1;
          if (pendingCommandsRef.current === 0 && isMountedRef.current) {
            setIsBusy(false);
          }
        }
      });
    },
    [acceptSnapshot, reportError],
  );

  const splitPane = useCallback(
    (direction: Direction): void => runCommand(() => desktopService.splitPane(direction)),
    [runCommand],
  );
  const focusPane = useCallback(
    (direction: Direction): void => runCommand(() => desktopService.focusPane(direction)),
    [runCommand],
  );
  const resizePane = useCallback(
    (direction: Direction): void => runCommand(() => desktopService.resizePane(direction)),
    [runCommand],
  );
  const closePane = useCallback(
    (): void => runCommand(() => desktopService.closePane()),
    [runCommand],
  );
  const toggleZoom = useCallback(
    (): void => runCommand(() => desktopService.toggleZoom()),
    [runCommand],
  );
  const createTab = useCallback(
    (): void => runCommand(() => desktopService.createTab()),
    [runCommand],
  );
  const activateTab = useCallback(
    (tabId: string): void => runCommand(() => desktopService.activateTab(tabId)),
    [runCommand],
  );
  const closeTab = useCallback(
    (tabId: string): void => runCommand(() => desktopService.closeTab(tabId)),
    [runCommand],
  );

  const cycleTab = useCallback(
    (offset: number): void => {
      runCommand((currentSnapshot) => {
        const { tabs, activeTabId } = currentSnapshot.session;
        const activeIndex = Math.max(
          0,
          tabs.findIndex((tab) => tab.id === activeTabId),
        );
        const nextIndex = (activeIndex + offset + tabs.length) % tabs.length;
        return desktopService.activateTab(tabs[nextIndex].id);
      });
    },
    [runCommand],
  );

  const activateTabAtIndex = useCallback(
    (index: number): void => {
      runCommand((currentSnapshot) => {
        const target = currentSnapshot.session.tabs[index];
        return target ? desktopService.activateTab(target.id) : Promise.resolve(currentSnapshot);
      });
    },
    [runCommand],
  );

  const activatePane = useCallback(
    (targetPaneId: string): void => {
      const currentSnapshot = snapshotRef.current;
      if (currentSnapshot && getActiveTab(currentSnapshot).activePaneId === targetPaneId) {
        return;
      }

      runCommand(async (currentSnapshot) => {
        let workingSnapshot = currentSnapshot;
        const tab = getActiveTab(workingSnapshot);
        const directions = focusPathToPane(tab.root, tab.activePaneId, targetPaneId);

        for (const direction of directions) {
          workingSnapshot = await desktopService.focusPane(direction);
          if (getActiveTab(workingSnapshot).activePaneId === targetPaneId) {
            break;
          }
        }

        return workingSnapshot;
      });
    },
    [runCommand],
  );

  const handlePrefixCommand = useCallback(
    (command: PrefixCommand): void => {
      switch (command.type) {
        case "focus":
          focusPane(command.direction);
          return;
        case "split":
          splitPane(command.direction);
          return;
        case "resize":
          resizePane(command.direction);
          return;
        case "createTab":
          createTab();
          return;
        case "nextTab":
          cycleTab(1);
          return;
        case "previousTab":
          cycleTab(-1);
          return;
        case "closePane":
          closePane();
          return;
        case "toggleZoom":
          toggleZoom();
          return;
        case "activateTab":
          activateTabAtIndex(command.index);
          return;
        case "cancel":
        case "unknown":
          return;
      }
    },
    [
      activateTabAtIndex,
      closePane,
      createTab,
      cycleTab,
      focusPane,
      resizePane,
      splitPane,
      toggleZoom,
    ],
  );
  const isPrefixActive = usePrefixShortcuts({ onCommand: handlePrefixCommand });

  if (!snapshot) {
    return (
      <main className="app-loading">
        <div className="app-loading__mark" aria-hidden="true">
          W+
        </div>
        <div>
          <strong>{isLoading ? "正在连接 Rust Core" : "无法载入终端状态"}</strong>
          <span>{error?.message ?? "正在准备本地会话…"}</span>
        </div>
        {!isLoading ? (
          <button type="button" onClick={() => void refreshSnapshot()}>
            重新连接
          </button>
        ) : null}
      </main>
    );
  }

  const tab = getActiveTab(snapshot);
  const activePane = findPane(tab.root, tab.activePaneId) ?? firstPane(tab);

  return (
    <main className="app-shell">
      <TabBar
        session={snapshot.session}
        onActivate={activateTab}
        onClose={closeTab}
        onCreate={createTab}
      />
      <div className="workspace-canvas">
        {error ? (
          <ErrorNotice
            error={error}
            onDismiss={() => setError(null)}
            onRetry={error.retryable ? () => void refreshSnapshot() : undefined}
          />
        ) : null}
        <SplitPaneTree
          root={tab.root}
          activePaneId={tab.activePaneId}
          zoomedPaneId={tab.zoomedPaneId}
          onActivate={activatePane}
          onSplit={splitPane}
          onClose={closePane}
          onToggleZoom={toggleZoom}
          onError={reportError}
          onStateChange={synchronizeSnapshot}
        />
      </div>
      <StatusBar
        snapshot={snapshot}
        activePane={activePane}
        isPrefixActive={isPrefixActive}
        isDesktop={desktopService.isDesktop}
        isBusy={isBusy}
      />
    </main>
  );
}

function getActiveTab(snapshot: AppSnapshot): TabSnapshot {
  return (
    snapshot.session.tabs.find((tab) => tab.id === snapshot.session.activeTabId) ??
    snapshot.session.tabs[0]
  );
}

function firstPane(tab: TabSnapshot) {
  const paneId = paneIdsInLayout(tab.root)[0];
  const pane = findPane(tab.root, paneId);
  if (!pane) {
    throw new Error("当前标签页没有可用窗格");
  }
  return pane;
}

function toAppError(value: unknown): AppError {
  if (value instanceof DesktopServiceError) {
    return value.appError;
  }

  return {
    code: "UI_RUNTIME_ERROR",
    message: value instanceof Error ? value.message : "界面操作失败",
    retryable: true,
  };
}

export default App;
