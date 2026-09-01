import { useEffect, useRef, useState } from "react";
import { isPrefixKey, resolvePrefixCommand, type PrefixCommand } from "../keyboard/prefix";

const DEFAULT_PREFIX_TIMEOUT_MS = 1_800;

export interface PrefixShortcutHandlers {
  readonly onCommand: (command: PrefixCommand) => void;
}

export function usePrefixShortcuts(
  handlers: PrefixShortcutHandlers,
  timeoutMs = DEFAULT_PREFIX_TIMEOUT_MS,
): boolean {
  const [isPrefixActive, setIsPrefixActive] = useState(false);
  const isPrefixActiveRef = useRef(false);
  const handlerRef = useRef(handlers.onCommand);
  const timeoutRef = useRef<number | null>(null);

  useEffect(() => {
    handlerRef.current = handlers.onCommand;
  }, [handlers.onCommand]);

  useEffect(() => {
    const cancelPrefix = (): void => {
      isPrefixActiveRef.current = false;
      setIsPrefixActive(false);
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
    };

    const activatePrefix = (): void => {
      cancelPrefix();
      isPrefixActiveRef.current = true;
      setIsPrefixActive(true);
      timeoutRef.current = window.setTimeout(cancelPrefix, timeoutMs);
    };

    const handleKeyDown = (event: KeyboardEvent): void => {
      if (isPrefixKey(event)) {
        event.preventDefault();
        event.stopPropagation();
        activatePrefix();
        return;
      }

      if (!isPrefixActiveRef.current) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      const command = resolvePrefixCommand(event);
      cancelPrefix();
      if (command.type !== "cancel" && command.type !== "unknown") {
        handlerRef.current(command);
      }
    };

    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
      isPrefixActiveRef.current = false;
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
    };
  }, [timeoutMs]);

  return isPrefixActive;
}
