import { findPane } from "../domain/layout";
import type { Direction, LayoutNode } from "../types/workspace";
import { TerminalPane } from "./TerminalPane";

interface SplitPaneTreeProps {
  readonly root: LayoutNode;
  readonly activePaneId: string;
  readonly zoomedPaneId: string | null;
  readonly onActivate: (paneId: string) => void;
  readonly onSplit: (direction: Direction) => void;
  readonly onClose: () => void;
  readonly onToggleZoom: () => void;
  readonly onError: (error: unknown) => void;
  readonly onStateChange: () => void;
}

export function SplitPaneTree(props: SplitPaneTreeProps): React.JSX.Element {
  const zoomedPane = props.zoomedPaneId ? findPane(props.root, props.zoomedPaneId) : null;
  if (zoomedPane) {
    return (
      <div className="pane-tree pane-tree--zoomed" data-testid="pane-tree">
        <TerminalPane
          pane={zoomedPane}
          isActive={true}
          isZoomed={true}
          onActivate={props.onActivate}
          onSplit={props.onSplit}
          onClose={props.onClose}
          onToggleZoom={props.onToggleZoom}
          onError={props.onError}
          onStateChange={props.onStateChange}
        />
      </div>
    );
  }

  return (
    <div className="pane-tree" data-testid="pane-tree">
      <LayoutBranch node={props.root} {...props} />
    </div>
  );
}

interface LayoutBranchProps extends SplitPaneTreeProps {
  readonly node: LayoutNode;
}

function LayoutBranch({ node, ...props }: LayoutBranchProps): React.JSX.Element {
  if (node.kind === "pane") {
    return (
      <TerminalPane
        pane={node.pane}
        isActive={node.pane.id === props.activePaneId}
        isZoomed={false}
        onActivate={props.onActivate}
        onSplit={props.onSplit}
        onClose={props.onClose}
        onToggleZoom={props.onToggleZoom}
        onError={props.onError}
        onStateChange={props.onStateChange}
      />
    );
  }

  const ratio = Math.min(0.9, Math.max(0.1, node.ratio));
  return (
    <div className={`pane-split pane-split--${node.axis}`}>
      <div className="pane-split__branch" style={{ flexBasis: `${ratio * 100}%` }}>
        <LayoutBranch node={node.first} {...props} />
      </div>
      <div className="pane-split__divider" aria-hidden="true" />
      <div className="pane-split__branch" style={{ flexBasis: `${(1 - ratio) * 100}%` }}>
        <LayoutBranch node={node.second} {...props} />
      </div>
    </div>
  );
}
