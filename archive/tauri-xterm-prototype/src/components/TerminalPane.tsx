import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { desktopService, type TerminalSubscription } from "../services/desktop";
import { createTerminalInputBuffer } from "../terminal/terminalInputBuffer";
import type {
  Direction,
  PaneSnapshot,
  TerminalDimensions,
  TerminalEvent,
} from "../types/workspace";
import { DirectionCross } from "./DirectionCross";
import { PaneToolbar } from "./PaneToolbar";

interface TerminalPaneProps {
  readonly pane: PaneSnapshot;
  readonly isActive: boolean;
  readonly isZoomed: boolean;
  readonly onActivate: (paneId: string) => void;
  readonly onSplit: (direction: Direction) => void;
  readonly onClose: () => void;
  readonly onToggleZoom: () => void;
  readonly onError: (error: unknown) => void;
  readonly onStateChange: () => void;
}

const TERMINAL_THEME = {
  background: "#10151b",
  foreground: "#d9e2ec",
  cursor: "#e6b566",
  cursorAccent: "#10151b",
  selectionBackground: "#33424f",
  black: "#10151b",
  red: "#d98178",
  green: "#63b3a6",
  yellow: "#e6b566",
  blue: "#7fa8c9",
  magenta: "#b69ac7",
  cyan: "#72b6c2",
  white: "#d9e2ec",
  brightBlack: "#71808f",
  brightRed: "#ee978e",
  brightGreen: "#7bc9bb",
  brightYellow: "#f1c984",
  brightBlue: "#9ac2e2",
  brightMagenta: "#ccb0dd",
  brightCyan: "#8dced8",
  brightWhite: "#f2f6f9",
} as const;

export function TerminalPane({
  pane,
  isActive,
  isZoomed,
  onActivate,
  onSplit,
  onClose,
  onToggleZoom,
  onError,
  onStateChange,
}: TerminalPaneProps): React.JSX.Element {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const shouldRestartRef = useRef(false);
  const [restartVersion, setRestartVersion] = useState(0);
  const [runtimeStatus, setRuntimeStatus] = useState(pane.status);
  const [runtimeMessage, setRuntimeMessage] = useState<string | null>(pane.statusMessage ?? null);

  const requestRestart = (): void => {
    shouldRestartRef.current = true;
    setRuntimeStatus("starting");
    setRuntimeMessage(null);
    setRestartVersion((version) => version + 1);
  };

  useEffect(() => {
    const host = hostRef.current;
    if (!host) {
      return;
    }

    let isDisposed = false;
    let subscription: TerminalSubscription | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let animationFrameId: number | null = null;
    let lastDimensions: TerminalDimensions | null = null;
    let lastSequence = -1;
    const shouldRestart = shouldRestartRef.current;
    shouldRestartRef.current = false;

    const terminal = new Terminal({
      allowTransparency: false,
      cursorBlink: true,
      cursorStyle: "block",
      drawBoldTextInBrightColors: true,
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
      fontSize: 13,
      letterSpacing: 0,
      lineHeight: 1.22,
      rightClickSelectsWord: true,
      scrollback: 10_000,
      theme: TERMINAL_THEME,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(host);
    terminalRef.current = terminal;
    host
      .querySelector(".xterm-helper-textarea")
      ?.setAttribute("aria-label", `${pane.title} 终端输入`);

    const reportError = (error: unknown): void => {
      if (!isDisposed) {
        setRuntimeStatus("error");
        setRuntimeMessage(error instanceof Error ? error.message : "终端通信失败");
        onError(error);
      }
    };

    const inputBuffer = createTerminalInputBuffer(
      (data) => desktopService.writeTerminal(pane.id, data),
      reportError,
    );
    const inputDisposable = terminal.onData((data) => inputBuffer.push(data));

    const receiveTerminalEvent = (event: TerminalEvent): void => {
      if (isDisposed || event.paneId !== pane.id || event.sequence <= lastSequence) {
        return;
      }

      lastSequence = event.sequence;
      if (event.kind === "output") {
        if (event.data) {
          terminal.write(event.data);
        }
        setRuntimeStatus("running");
        setRuntimeMessage(null);
        return;
      }

      if (event.kind === "exited") {
        setRuntimeStatus("exited");
        setRuntimeMessage(
          `进程已退出${event.exitCode === undefined ? "" : ` · ${event.exitCode}`}`,
        );
        terminal.writeln(
          `\r\n\u001b[38;2;230;181;102m[process exited${event.exitCode === undefined ? "" : `: ${event.exitCode}`} ]\u001b[0m`,
        );
        onStateChange();
        return;
      }

      setRuntimeStatus("error");
      setRuntimeMessage(event.data || "终端读取失败");
      onStateChange();
    };

    const currentDimensions = (): TerminalDimensions => ({
      rows: Math.max(2, terminal.rows),
      cols: Math.max(2, terminal.cols),
    });

    const fitAndResize = (): void => {
      animationFrameId = null;
      if (isDisposed || host.clientWidth <= 0 || host.clientHeight <= 0) {
        return;
      }

      try {
        fitAddon.fit();
        const dimensions = currentDimensions();
        if (
          subscription &&
          (dimensions.rows !== lastDimensions?.rows || dimensions.cols !== lastDimensions?.cols)
        ) {
          lastDimensions = dimensions;
          void desktopService.resizeTerminal(pane.id, dimensions).catch(reportError);
        }
      } catch (error: unknown) {
        reportError(error);
      }
    };

    const scheduleFit = (): void => {
      if (animationFrameId !== null) {
        window.cancelAnimationFrame(animationFrameId);
      }
      animationFrameId = window.requestAnimationFrame(fitAndResize);
    };

    const connect = async (): Promise<void> => {
      try {
        const dimensions = currentDimensions();
        const nextSubscription = shouldRestart
          ? await desktopService.restartTerminal(pane.id, dimensions, receiveTerminalEvent)
          : await desktopService.startTerminal(pane.id, dimensions, receiveTerminalEvent);
        if (isDisposed) {
          nextSubscription.dispose();
          return;
        }

        subscription = nextSubscription;
        lastDimensions = dimensions;
        setRuntimeStatus("running");
        onStateChange();
        scheduleFit();
      } catch (error: unknown) {
        reportError(error);
        onStateChange();
      }
    };

    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(scheduleFit);
      resizeObserver.observe(host);
    }
    void connect();

    return () => {
      isDisposed = true;
      if (animationFrameId !== null) {
        window.cancelAnimationFrame(animationFrameId);
      }
      resizeObserver?.disconnect();
      inputBuffer.dispose();
      inputDisposable.dispose();
      subscription?.dispose();
      fitAddon.dispose();
      terminal.dispose();
      if (terminalRef.current === terminal) {
        terminalRef.current = null;
      }
    };
  }, [onError, onStateChange, pane.id, pane.title, restartVersion]);

  useEffect(() => {
    if (isActive) {
      terminalRef.current?.focus();
    }
  }, [isActive]);

  return (
    <section
      className={`terminal-pane${isActive ? " terminal-pane--active" : ""}`}
      aria-label={`${pane.title}${isActive ? "，活动窗格" : ""}`}
      data-pane-id={pane.id}
      onPointerDown={() => onActivate(pane.id)}
    >
      <PaneToolbar
        pane={pane}
        isActive={isActive}
        isZoomed={isZoomed}
        runtimeStatus={runtimeStatus}
        runtimeMessage={runtimeMessage}
        onSplit={onSplit}
        onClose={onClose}
        onToggleZoom={onToggleZoom}
        onRestart={requestRestart}
      />
      {isActive ? <DirectionCross /> : null}
      <div className="terminal-pane__viewport" ref={hostRef} />
      {runtimeStatus === "error" ? (
        <div className="terminal-pane__state" role="status">
          <span>{runtimeMessage || "终端启动失败"}</span>
          <button type="button" onClick={requestRestart}>
            重新启动
          </button>
        </div>
      ) : null}
    </section>
  );
}
