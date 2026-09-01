import { describe, expect, it } from "vitest";
import {
  calculatePaneRects,
  focusPathToPane,
  nearestPaneId,
  removePaneFromLayout,
  resizeLayout,
  splitLayout,
} from "./layout";
import type { LayoutNode, PaneSnapshot } from "../types/workspace";

const pane = (id: string): PaneSnapshot => ({
  id,
  title: id,
  profileId: "powershell",
  status: "running",
});

const paneNode = (id: string): LayoutNode => ({ kind: "pane", pane: pane(id) });

describe("layout helpers", () => {
  it("places a new pane on the requested side", () => {
    const left = splitLayout(paneNode("current"), "current", pane("new"), "left");
    const down = splitLayout(paneNode("current"), "current", pane("new"), "down");

    expect(left).toMatchObject({
      kind: "split",
      axis: "row",
      first: { kind: "pane", pane: { id: "new" } },
      second: { kind: "pane", pane: { id: "current" } },
    });
    expect(down).toMatchObject({
      kind: "split",
      axis: "column",
      first: { kind: "pane", pane: { id: "current" } },
      second: { kind: "pane", pane: { id: "new" } },
    });
  });

  it("navigates an irregular split by rendered geometry", () => {
    const layout = irregularLayout();

    expect(nearestPaneId(layout, "left", "right")).toBe("right-bottom");
    expect(nearestPaneId(layout, "right-top", "down")).toBe("right-bottom");
    expect(focusPathToPane(layout, "left", "right-top")).toEqual(["up"]);
  });

  it("calculates normalized pane rectangles", () => {
    const rects = calculatePaneRects(irregularLayout());

    expect(rects.get("left")).toEqual({ x: 0, y: 0, width: 0.6, height: 1 });
    expect(rects.get("right-bottom")).toEqual({ x: 0.6, y: 0.4, width: 0.4, height: 0.6 });
  });

  it("resizes the nearest matching ancestor and clamps ratios", () => {
    const resized = resizeLayout(irregularLayout(), "right-top", "down", 0.2);
    const repeatedlyResized = resizeLayout(resized, "right-top", "down", 0.8);

    expect(
      resized.kind === "split" && resized.second.kind === "split" && resized.second.ratio,
    ).toBeCloseTo(0.6);
    expect(repeatedlyResized).toMatchObject({ second: { ratio: 0.9 } });
  });

  it("promotes the sibling when a pane closes", () => {
    const remaining = removePaneFromLayout(irregularLayout(), "right-top");

    expect(remaining).toMatchObject({
      kind: "split",
      axis: "row",
      second: { kind: "pane", pane: { id: "right-bottom" } },
    });
  });
});

function irregularLayout(): LayoutNode {
  return {
    kind: "split",
    axis: "row",
    ratio: 0.6,
    first: paneNode("left"),
    second: {
      kind: "split",
      axis: "column",
      ratio: 0.4,
      first: paneNode("right-top"),
      second: paneNode("right-bottom"),
    },
  };
}
