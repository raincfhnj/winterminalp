import type { Direction } from "../types/workspace";

export type PrefixCommand =
  | { readonly type: "focus"; readonly direction: Direction }
  | { readonly type: "split"; readonly direction: Direction }
  | { readonly type: "resize"; readonly direction: Direction }
  | { readonly type: "createTab" }
  | { readonly type: "nextTab" }
  | { readonly type: "previousTab" }
  | { readonly type: "closePane" }
  | { readonly type: "toggleZoom" }
  | { readonly type: "activateTab"; readonly index: number }
  | { readonly type: "cancel" }
  | { readonly type: "unknown" };

export interface PrefixKey {
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
}

const DIRECTIONS: Readonly<Record<string, Direction>> = {
  left: "left",
  right: "right",
  up: "up",
  down: "down",
};

export function isPrefixKey(key: PrefixKey): boolean {
  return key.ctrlKey && !key.altKey && !key.metaKey && key.key.toLocaleLowerCase() === "b";
}

export function resolvePrefixCommand(key: PrefixKey): PrefixCommand {
  if (key.key === "Escape") {
    return { type: "cancel" };
  }

  const direction = DIRECTIONS[key.key.toLocaleLowerCase().replace(/^arrow/, "")];
  if (direction) {
    if (key.shiftKey) {
      return { type: "split", direction };
    }

    if (key.ctrlKey) {
      return { type: "resize", direction };
    }

    return { type: "focus", direction };
  }

  if (/^[0-9]$/.test(key.key)) {
    return { type: "activateTab", index: Number(key.key) };
  }

  switch (key.key.toLocaleLowerCase()) {
    case "c":
      return { type: "createTab" };
    case "n":
      return { type: "nextTab" };
    case "p":
      return { type: "previousTab" };
    case "x":
      return { type: "closePane" };
    case "z":
      return { type: "toggleZoom" };
    default:
      return { type: "unknown" };
  }
}
