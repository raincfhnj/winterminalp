// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { createTerminalInputBuffer } from "./terminalInputBuffer";

afterEach(() => {
  vi.useRealTimers();
});

describe("terminal input buffer", () => {
  it("batches adjacent input without reordering it", async () => {
    vi.useFakeTimers();
    const write = vi.fn(async () => undefined);
    const onError = vi.fn();
    const buffer = createTerminalInputBuffer(write, onError, 10);

    buffer.push("a");
    buffer.push("b");
    expect(write).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(10);
    expect(write).toHaveBeenCalledWith("ab");
    expect(onError).not.toHaveBeenCalled();
  });

  it("flushes Enter immediately and flushes pending data on cleanup", async () => {
    const write = vi.fn(async () => undefined);
    const buffer = createTerminalInputBuffer(write, vi.fn(), 50);

    buffer.push("command");
    buffer.push("\r");
    await vi.waitFor(() => expect(write).toHaveBeenCalledWith("command\r"));

    buffer.push("next");
    buffer.dispose();
    await vi.waitFor(() => expect(write).toHaveBeenLastCalledWith("next"));
  });

  it("serializes batches so IPC completion cannot reorder terminal input", async () => {
    let releaseFirst: (() => void) | undefined;
    const write = vi
      .fn<(data: string) => Promise<void>>()
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            releaseFirst = resolve;
          }),
      )
      .mockResolvedValue(undefined);
    const buffer = createTerminalInputBuffer(write, vi.fn(), 50);

    buffer.push("first\r");
    buffer.push("second\r");
    await vi.waitFor(() => expect(write).toHaveBeenCalledTimes(1));
    expect(write).toHaveBeenNthCalledWith(1, "first\r");

    releaseFirst?.();
    await vi.waitFor(() => expect(write).toHaveBeenCalledTimes(2));
    expect(write).toHaveBeenNthCalledWith(2, "second\r");
  });
});
