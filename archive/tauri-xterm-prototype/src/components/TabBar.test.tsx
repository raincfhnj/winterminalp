// @vitest-environment jsdom

import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";
import type { SessionSnapshot } from "../types/workspace";

const session: SessionSnapshot = {
  id: "session-1",
  name: "local workspace",
  activeTabId: "tab-1",
  tabs: [
    {
      id: "tab-1",
      title: "workspace",
      activePaneId: "pane-1",
      zoomedPaneId: null,
      root: {
        kind: "pane",
        pane: {
          id: "pane-1",
          title: "PowerShell",
          profileId: "powershell",
          status: "running",
        },
      },
    },
    {
      id: "tab-2",
      title: "logs",
      activePaneId: "pane-2",
      zoomedPaneId: null,
      root: {
        kind: "pane",
        pane: {
          id: "pane-2",
          title: "PowerShell",
          profileId: "powershell",
          status: "running",
        },
      },
    },
  ],
};

describe("TabBar", () => {
  it("exposes active state and dispatches tab actions", () => {
    const onActivate = vi.fn();
    const onClose = vi.fn();
    const onCreate = vi.fn();
    render(
      <TabBar session={session} onActivate={onActivate} onClose={onClose} onCreate={onCreate} />,
    );

    expect(screen.getByRole("tab", { name: /workspace/ })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("tab", { name: /logs/ }));
    fireEvent.click(screen.getByRole("button", { name: "关闭 logs" }));
    fireEvent.click(screen.getByRole("button", { name: "新建标签页" }));

    expect(onActivate).toHaveBeenCalledWith("tab-2");
    expect(onClose).toHaveBeenCalledWith("tab-2");
    expect(onCreate).toHaveBeenCalledOnce();
  });
});
