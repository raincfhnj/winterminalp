import type { AppError } from "../types/workspace";

interface ErrorNoticeProps {
  readonly error: AppError;
  readonly onDismiss: () => void;
  readonly onRetry?: () => void;
}

export function ErrorNotice({ error, onDismiss, onRetry }: ErrorNoticeProps): React.JSX.Element {
  return (
    <aside className="error-notice" role="alert">
      <span className="error-notice__mark" aria-hidden="true">
        !
      </span>
      <div className="error-notice__copy">
        <strong>{error.message}</strong>
        {error.detail ? <span>{error.detail}</span> : null}
      </div>
      <code>{error.code}</code>
      {error.retryable && onRetry ? (
        <button type="button" onClick={onRetry}>
          重试
        </button>
      ) : null}
      <button
        className="error-notice__dismiss"
        type="button"
        aria-label="关闭错误提示"
        onClick={onDismiss}
      >
        ×
      </button>
    </aside>
  );
}
