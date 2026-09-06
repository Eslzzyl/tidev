import { Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Session } from "../../types/api";
import { IconButton } from "../ui";

export interface SessionTabBarProps {
  openSessionIds: string[];
  sessions: Session[];
  selectedSessionId: string | null;
  completedSessions: Set<string>;
  onSelectSession: (sessionId: string) => void;
  onCloseTab: (sessionId: string) => void;
  onCreateSession: () => void;
}

export function SessionTabBar({
  openSessionIds,
  sessions,
  selectedSessionId,
  completedSessions,
  onSelectSession,
  onCloseTab,
  onCreateSession,
}: SessionTabBarProps) {
  const { t } = useTranslation();

  if (openSessionIds.length === 0) return null;

  return (
    <nav className="session-tab-bar" aria-label={t("Open conversations")}>
      <div className="session-tab-list">
        {openSessionIds.map((id) => {
          const session = sessions.find((s) => s.session_id === id);
          const isSelected = selectedSessionId === id;
          const isBusy = session?.busy ?? false;
          const isCompleted = completedSessions.has(id);
          const title = session?.title || t("Untitled conversation");

          return (
            <div
              key={id}
              className={`session-tab ${isSelected ? "selected" : ""}`}
              onClick={() => onSelectSession(id)}
              role="tab"
              aria-selected={isSelected}
              title={title}
            >
              {isBusy ? (
                <span className="session-busy-spinner" aria-label={t("Running")} />
              ) : isCompleted ? (
                <span className="session-completed-dot" aria-label={t("Completed")} />
              ) : null}
              <span className="session-tab-title">{title}</span>
              <button
                type="button"
                className="session-tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  onCloseTab(id);
                }}
                aria-label={t("Close tab")}
                title={t("Close tab")}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <IconButton
        label={t("New conversation")}
        size="sm"
        className="session-tab-new-button"
        onClick={onCreateSession}
        title={t("New conversation")}
      >
        <Plus size={14} />
      </IconButton>
    </nav>
  );
}
