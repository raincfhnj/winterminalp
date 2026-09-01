// @vitest-environment jsdom

import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SplitPaneTree } from "./SplitPaneTree";
import type { LayoutNode } from "../types/workspace";

vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ pane, isActive }: { pane: { id: string }; isActive: boolean }) => (
    <div data-testid="terminal-pane" data-pane-id={pane.id} data-active={isActive} />
  ),
}));

const root: LayoutNode = {
  kind: "split",
  axis: "row",
  ratio: 0.6,
  first: {
    kind: "pane",
    pane: { id: "left", title: "left", profileId: "powershell", status: "running" },
  },
  second: {
    kind: "split",
    axis: "column",
    ratio: 0.5,
    first: {
      kind: "pane",
      pane: { id: "top", title: "top", profileId: "powershell", status: "running" },
    },
    second: {
      kind: "pane",
      pane: { id: "bottom", title: "bottom", profileId: "powershell", status: "running" },
    },
  },
};

const handlers = {
  onActivate: vi.fn(),
  onSplit: vi.fn(),
  onClose: vi.fn(),
  onToggleZoom: vi.fn(),
  onError: vi.fn(),
  onStateChange: vi.fn(),
};

afterEach(cleanup);

describe("SplitPaneTree", () => {
  it("renders each recursive leaf and marks the active pane", () => {
    render(<SplitPaneTree root={root} activePaneId="top" zoomedPaneId={null} {...handlers} />);

    expect(screen.getAllByTestId("terminal-pane")).toHaveLength(3);
    expect(document.querySelector('[data-pane-id="top"]')).toHaveAttribute("data-active", "true");
  });

  it("renders only the selected pane while zoomed", () => {
    render(<SplitPaneTree root={root} activePaneId="bottom" zoomedPaneId="bottom" {...handlers} />);

    expect(screen.getAllByTestId("terminal-pane")).toHaveLength(1);
    expect(document.querySelector('[data-pane-id="bottom"]')).toBeInTheDocument();
  });
});
