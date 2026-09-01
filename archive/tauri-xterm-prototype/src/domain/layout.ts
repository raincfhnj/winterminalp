import type { Direction, LayoutNode, PaneSnapshot, SplitAxis } from "../types/workspace";

const MIN_SPLIT_RATIO = 0.1;
const MAX_SPLIT_RATIO = 0.9;
const DEFAULT_RESIZE_STEP = 0.05;
const NAVIGATION_ORDER: readonly Direction[] = ["left", "right", "up", "down"];

export interface PaneRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

interface PositionedPane {
  readonly paneId: string;
  readonly rect: PaneRect;
}

interface LayoutPathEntry {
  readonly node: LayoutNode & { readonly kind: "split" };
  readonly branch: "first" | "second";
}

export function paneIdsInLayout(node: LayoutNode): readonly string[] {
  if (node.kind === "pane") {
    return [node.pane.id];
  }

  return [...paneIdsInLayout(node.first), ...paneIdsInLayout(node.second)];
}

export function paneSnapshotsInLayout(node: LayoutNode): readonly PaneSnapshot[] {
  if (node.kind === "pane") {
    return [node.pane];
  }

  return [...paneSnapshotsInLayout(node.first), ...paneSnapshotsInLayout(node.second)];
}

export function containsPane(node: LayoutNode, paneId: string): boolean {
  if (node.kind === "pane") {
    return node.pane.id === paneId;
  }

  return containsPane(node.first, paneId) || containsPane(node.second, paneId);
}

export function findPane(node: LayoutNode, paneId: string): PaneSnapshot | null {
  if (node.kind === "pane") {
    return node.pane.id === paneId ? node.pane : null;
  }

  return findPane(node.first, paneId) ?? findPane(node.second, paneId);
}

export function splitLayout(
  node: LayoutNode,
  targetPaneId: string,
  newPane: PaneSnapshot,
  direction: Direction,
): LayoutNode {
  if (node.kind === "pane") {
    if (node.pane.id !== targetPaneId) {
      return node;
    }

    const newPaneNode: LayoutNode = { kind: "pane", pane: newPane };
    const axis: SplitAxis = direction === "left" || direction === "right" ? "row" : "column";
    const isNewPaneFirst = direction === "left" || direction === "up";

    return {
      kind: "split",
      axis,
      ratio: 0.5,
      first: isNewPaneFirst ? newPaneNode : node,
      second: isNewPaneFirst ? node : newPaneNode,
    };
  }

  if (containsPane(node.first, targetPaneId)) {
    return {
      ...node,
      first: splitLayout(node.first, targetPaneId, newPane, direction),
    };
  }

  if (containsPane(node.second, targetPaneId)) {
    return {
      ...node,
      second: splitLayout(node.second, targetPaneId, newPane, direction),
    };
  }

  return node;
}

export function removePaneFromLayout(node: LayoutNode, paneId: string): LayoutNode | null {
  if (node.kind === "pane") {
    return node.pane.id === paneId ? null : node;
  }

  const first = removePaneFromLayout(node.first, paneId);
  const second = removePaneFromLayout(node.second, paneId);

  if (!first) {
    return second;
  }

  if (!second) {
    return first;
  }

  return { ...node, first, second };
}

export function calculatePaneRects(
  node: LayoutNode,
  bounds: PaneRect = { x: 0, y: 0, width: 1, height: 1 },
): ReadonlyMap<string, PaneRect> {
  const panes: PositionedPane[] = [];
  collectPositionedPanes(node, bounds, panes);
  return new Map(panes.map(({ paneId, rect }) => [paneId, rect]));
}

export function nearestPaneId(
  node: LayoutNode,
  activePaneId: string,
  direction: Direction,
): string | null {
  const rects = calculatePaneRects(node);
  const activeRect = rects.get(activePaneId);

  if (!activeRect) {
    return null;
  }

  const candidates = [...rects.entries()]
    .filter(
      ([paneId, rect]) => paneId !== activePaneId && isInDirection(activeRect, rect, direction),
    )
    .map(([paneId, rect]) => ({ paneId, score: directionalScore(activeRect, rect, direction) }))
    .sort((left, right) => left.score - right.score || left.paneId.localeCompare(right.paneId));

  return candidates[0]?.paneId ?? null;
}

export function focusPathToPane(
  node: LayoutNode,
  activePaneId: string,
  targetPaneId: string,
): readonly Direction[] {
  if (activePaneId === targetPaneId) {
    return [];
  }

  const queue: Array<{ readonly paneId: string; readonly path: readonly Direction[] }> = [
    { paneId: activePaneId, path: [] },
  ];
  const visited = new Set([activePaneId]);

  while (queue.length > 0) {
    const current = queue.shift();
    if (!current) {
      break;
    }

    for (const direction of NAVIGATION_ORDER) {
      const nextPaneId = nearestPaneId(node, current.paneId, direction);
      if (!nextPaneId || visited.has(nextPaneId)) {
        continue;
      }

      const path = [...current.path, direction];
      if (nextPaneId === targetPaneId) {
        return path;
      }

      visited.add(nextPaneId);
      queue.push({ paneId: nextPaneId, path });
    }
  }

  return [];
}

export function resizeLayout(
  node: LayoutNode,
  activePaneId: string,
  direction: Direction,
  step = DEFAULT_RESIZE_STEP,
): LayoutNode {
  const path: LayoutPathEntry[] = [];
  if (!findLayoutPath(node, activePaneId, path)) {
    return node;
  }

  for (let index = path.length - 1; index >= 0; index -= 1) {
    const entry = path[index];
    const delta = ratioDelta(entry.node.axis, entry.branch, direction, step);

    if (delta !== null) {
      return updateSplitRatio(node, entry.node, clampRatio(entry.node.ratio + delta));
    }
  }

  return node;
}

function collectPositionedPanes(node: LayoutNode, bounds: PaneRect, panes: PositionedPane[]): void {
  if (node.kind === "pane") {
    panes.push({ paneId: node.pane.id, rect: bounds });
    return;
  }

  const clampedRatio = clampRatio(node.ratio);
  if (node.axis === "row") {
    const firstWidth = bounds.width * clampedRatio;
    collectPositionedPanes(node.first, { ...bounds, width: firstWidth }, panes);
    collectPositionedPanes(
      node.second,
      {
        x: bounds.x + firstWidth,
        y: bounds.y,
        width: bounds.width - firstWidth,
        height: bounds.height,
      },
      panes,
    );
    return;
  }

  const firstHeight = bounds.height * clampedRatio;
  collectPositionedPanes(node.first, { ...bounds, height: firstHeight }, panes);
  collectPositionedPanes(
    node.second,
    {
      x: bounds.x,
      y: bounds.y + firstHeight,
      width: bounds.width,
      height: bounds.height - firstHeight,
    },
    panes,
  );
}

function isInDirection(active: PaneRect, candidate: PaneRect, direction: Direction): boolean {
  const activeCenter = centerOf(active);
  const candidateCenter = centerOf(candidate);

  switch (direction) {
    case "left":
      return candidateCenter.x < activeCenter.x;
    case "right":
      return candidateCenter.x > activeCenter.x;
    case "up":
      return candidateCenter.y < activeCenter.y;
    case "down":
      return candidateCenter.y > activeCenter.y;
  }
}

function directionalScore(active: PaneRect, candidate: PaneRect, direction: Direction): number {
  const activeCenter = centerOf(active);
  const candidateCenter = centerOf(candidate);
  const isHorizontal = direction === "left" || direction === "right";
  const primaryDistance = isHorizontal
    ? Math.abs(candidateCenter.x - activeCenter.x)
    : Math.abs(candidateCenter.y - activeCenter.y);
  const crossDistance = isHorizontal
    ? intervalDistance(
        active.y,
        active.y + active.height,
        candidate.y,
        candidate.y + candidate.height,
      )
    : intervalDistance(
        active.x,
        active.x + active.width,
        candidate.x,
        candidate.x + candidate.width,
      );
  const crossCenterDistance = isHorizontal
    ? Math.abs(candidateCenter.y - activeCenter.y)
    : Math.abs(candidateCenter.x - activeCenter.x);
  const crossOverlap = isHorizontal
    ? intervalOverlap(
        active.y,
        active.y + active.height,
        candidate.y,
        candidate.y + candidate.height,
      )
    : intervalOverlap(
        active.x,
        active.x + active.width,
        candidate.x,
        candidate.x + candidate.width,
      );
  const maximumCrossOverlap = isHorizontal
    ? Math.min(active.height, candidate.height)
    : Math.min(active.width, candidate.width);
  const overlapPenalty = maximumCrossOverlap > 0 ? 1 - crossOverlap / maximumCrossOverlap : 1;

  return primaryDistance + crossDistance * 2 + overlapPenalty * 0.75 + crossCenterDistance * 0.25;
}

function centerOf(rect: PaneRect): { readonly x: number; readonly y: number } {
  return {
    x: rect.x + rect.width / 2,
    y: rect.y + rect.height / 2,
  };
}

function intervalDistance(startA: number, endA: number, startB: number, endB: number): number {
  if (endA < startB) {
    return startB - endA;
  }

  if (endB < startA) {
    return startA - endB;
  }

  return 0;
}

function intervalOverlap(startA: number, endA: number, startB: number, endB: number): number {
  return Math.max(0, Math.min(endA, endB) - Math.max(startA, startB));
}

function findLayoutPath(node: LayoutNode, paneId: string, path: LayoutPathEntry[]): boolean {
  if (node.kind === "pane") {
    return node.pane.id === paneId;
  }

  path.push({ node, branch: "first" });
  if (findLayoutPath(node.first, paneId, path)) {
    return true;
  }
  path.pop();

  path.push({ node, branch: "second" });
  if (findLayoutPath(node.second, paneId, path)) {
    return true;
  }
  path.pop();

  return false;
}

function ratioDelta(
  axis: SplitAxis,
  branch: "first" | "second",
  direction: Direction,
  step: number,
): number | null {
  if (axis === "row" && branch === "first" && direction === "right") {
    return step;
  }

  if (axis === "row" && branch === "second" && direction === "left") {
    return -step;
  }

  if (axis === "column" && branch === "first" && direction === "down") {
    return step;
  }

  if (axis === "column" && branch === "second" && direction === "up") {
    return -step;
  }

  return null;
}

function updateSplitRatio(
  node: LayoutNode,
  target: LayoutNode & { readonly kind: "split" },
  ratio: number,
): LayoutNode {
  if (node === target) {
    return { ...node, ratio };
  }

  if (node.kind === "pane") {
    return node;
  }

  return {
    ...node,
    first: updateSplitRatio(node.first, target, ratio),
    second: updateSplitRatio(node.second, target, ratio),
  };
}

function clampRatio(ratio: number): number {
  return Math.min(MAX_SPLIT_RATIO, Math.max(MIN_SPLIT_RATIO, ratio));
}
