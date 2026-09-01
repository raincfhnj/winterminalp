export function DirectionCross(): React.JSX.Element {
  return (
    <div
      className="direction-cross"
      role="img"
      aria-label="活动窗格。按 Ctrl+B 后使用方向键移动焦点"
      title="Ctrl+B · 方向键"
    >
      <span className="direction-cross__arm direction-cross__arm--up">↑</span>
      <span className="direction-cross__arm direction-cross__arm--right">→</span>
      <span className="direction-cross__arm direction-cross__arm--down">↓</span>
      <span className="direction-cross__arm direction-cross__arm--left">←</span>
      <span className="direction-cross__center">B</span>
    </div>
  );
}
