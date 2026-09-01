import { describe, expect, it } from "vitest";
import { isPrefixKey, resolvePrefixCommand, type PrefixKey } from "./prefix";

const key = (overrides: Partial<PrefixKey>): PrefixKey => ({
  key: "",
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  metaKey: false,
  ...overrides,
});

describe("prefix keyboard mapping", () => {
  it("recognizes Ctrl+B without hijacking modified variants", () => {
    expect(isPrefixKey(key({ key: "b", ctrlKey: true }))).toBe(true);
    expect(isPrefixKey(key({ key: "b", ctrlKey: true, altKey: true }))).toBe(false);
  });

  it("maps plain, Shift and Ctrl arrows to focus, split and resize", () => {
    expect(resolvePrefixCommand(key({ key: "ArrowLeft" }))).toEqual({
      type: "focus",
      direction: "left",
    });
    expect(resolvePrefixCommand(key({ key: "ArrowUp", shiftKey: true }))).toEqual({
      type: "split",
      direction: "up",
    });
    expect(resolvePrefixCommand(key({ key: "ArrowDown", ctrlKey: true }))).toEqual({
      type: "resize",
      direction: "down",
    });
    expect(resolvePrefixCommand(key({ key: "Right" }))).toEqual({
      type: "focus",
      direction: "right",
    });
  });

  it("maps tab, pane, zoom and numeric commands", () => {
    expect(resolvePrefixCommand(key({ key: "C" }))).toEqual({ type: "createTab" });
    expect(resolvePrefixCommand(key({ key: "n" }))).toEqual({ type: "nextTab" });
    expect(resolvePrefixCommand(key({ key: "p" }))).toEqual({ type: "previousTab" });
    expect(resolvePrefixCommand(key({ key: "x" }))).toEqual({ type: "closePane" });
    expect(resolvePrefixCommand(key({ key: "z" }))).toEqual({ type: "toggleZoom" });
    expect(resolvePrefixCommand(key({ key: "7" }))).toEqual({ type: "activateTab", index: 7 });
    expect(resolvePrefixCommand(key({ key: "Escape" }))).toEqual({ type: "cancel" });
  });
});
