import type { SessionSnapshot } from "../types/workspace";

interface TabBarProps {
  readonly session: SessionSnapshot;
  readonly onActivate: (tabId: string) => void;
  readonly onClose: (tabId: string) => void;
  readonly onCreate: () => void;
}

export function TabBar({ session, onActivate, onClose, onCreate }: TabBarProps): React.JSX.Element {
  return (
    <header className="tab-bar">
      <div className="app-mark" data-tauri-drag-region>
        <span className="app-mark__monogram" aria-hidden="true">
          W+
        </span>
        <span className="app-mark__name">WinTerminal++</span>
      </div>

      <div className="tab-track" role="tablist" aria-label={`${session.name} 标签页`}>
        {session.tabs.map((tab, index) => {
          const isActive = tab.id === session.activeTabId;
          return (
            <div className={`tab-item${isActive ? " tab-item--active" : ""}`} key={tab.id}>
              <button
                className="tab-item__select"
                type="button"
                role="tab"
                aria-selected={isActive}
                tabIndex={isActive ? 0 : -1}
                onClick={() => onActivate(tab.id)}
              >
                <span className="tab-item__index" aria-hidden="true">
                  {index}
                </span>
                <span className="tab-item__title">{tab.title}</span>
              </button>
              <button
                className="tab-item__close"
                type="button"
                aria-label={`关闭 ${tab.title}`}
                title="关闭标签页"
                onClick={() => onClose(tab.id)}
              >
                ×
              </button>
            </div>
          );
        })}
        <button
          className="tab-add"
          type="button"
          aria-label="新建标签页"
          title="新建标签页 · Ctrl+B，C"
          onClick={onCreate}
        >
          +
        </button>
      </div>

      <div className="tab-bar__drag-space" data-tauri-drag-region aria-hidden="true" />
    </header>
  );
}
