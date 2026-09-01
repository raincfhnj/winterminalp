const DEFAULT_FLUSH_DELAY_MS = 8;
const MAX_BATCH_LENGTH = 64;

export interface TerminalInputBuffer {
  push(data: string): void;
  flush(): void;
  dispose(): void;
}

export function createTerminalInputBuffer(
  write: (data: string) => Promise<void>,
  onError: (error: unknown) => void,
  flushDelayMs = DEFAULT_FLUSH_DELAY_MS,
): TerminalInputBuffer {
  let pending = "";
  let timeoutId: number | null = null;
  let isDisposed = false;
  let writeQueue = Promise.resolve();

  const clearPendingTimeout = (): void => {
    if (timeoutId !== null) {
      window.clearTimeout(timeoutId);
      timeoutId = null;
    }
  };

  const flush = (): void => {
    clearPendingTimeout();
    if (!pending) {
      return;
    }

    const batch = pending;
    pending = "";
    writeQueue = writeQueue.then(() => write(batch)).catch(onError);
  };

  return {
    push(data: string) {
      if (isDisposed) {
        return;
      }

      pending += data;
      if (data.includes("\r") || pending.length >= MAX_BATCH_LENGTH) {
        flush();
        return;
      }

      if (timeoutId === null) {
        timeoutId = window.setTimeout(flush, flushDelayMs);
      }
    },
    flush,
    dispose() {
      if (isDisposed) {
        return;
      }

      isDisposed = true;
      flush();
      clearPendingTimeout();
    },
  };
}
