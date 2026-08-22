import { Pencil, Plus, Search, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Session } from "../../types/api";
import { formatDate, shortPath } from "../../utils/chat";

export interface SessionSidebarProps {
  loading: boolean;
  sessions: Session[];
  selectedSessionId: string | null;
  search: string;
  renamingSessionId: string | null;
  renameValue: string;
  onSearchChange: (value: string) => void;
  onCreate: () => void;
  onSelect: (sessionId: string) => void;
  onStartRename: (session: Session) => void;
  onRenameChange: (value: string) => void;
  onRename: (sessionId: string) => void;
  onCancelRename: () => void;
  onDelete: (session: Session) => void;
}

export function SessionSidebar({
  loading,
  sessions,
  selectedSessionId,
  search,
  renamingSessionId,
  renameValue,
  onSearchChange,
  onCreate,
  onSelect,
  onStartRename,
  onRenameChange,
  onRename,
  onCancelRename,
  onDelete,
}: SessionSidebarProps) {
  const { t } = useTranslation();
  const normalizedSearch = search.trim().toLowerCase();
  const visibleSessions = sessions.filter((session) =>
    session.title.toLowerCase().includes(normalizedSearch),
  );

  return (
    <aside className="session-sidebar">
      <div className="sidebar-heading">
        <div>
          <span className="eyebrow">{t("Workspace")}</span>
          <strong>{t("Conversations")}</strong>
        </div>
        <button className="icon-button" onClick={onCreate} title={t("New conversation")}>
          <Plus size={17} />
        </button>
      </div>
      <div className="session-search">
        <Search size={14} />
        <input
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={t("Search sessions…")}
          aria-label={t("Search sessions")}
        />
      </div>
      <div className="session-list">
        {loading ? <div className="empty-state">{t("Loading sessions…")}</div> : null}
        {!loading && sessions.length === 0 ? (
          <div className="empty-state">{t("No conversations yet.")}</div>
        ) : null}
        {visibleSessions.map((session) => (
          <div
            className={
              selectedSessionId === session.session_id ? "session-item selected" : "session-item"
            }
            key={session.session_id}
          >
            {renamingSessionId === session.session_id ? (
              <input
                className="session-rename-input"
                value={renameValue}
                onChange={(event) => onRenameChange(event.target.value)}
                onBlur={() => onRename(session.session_id)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void onRename(session.session_id);
                  if (event.key === "Escape") onCancelRename();
                }}
                autoFocus
              />
            ) : (
              <button
                className="session-select"
                onClick={() => onSelect(session.session_id)}
                onDoubleClick={() => onStartRename(session)}
              >
                <span className="session-title">{session.title || t("Untitled conversation")}</span>
                <span className="session-meta">
                  {session.busy ? <span className="busy-indicator" /> : null}
                  {session.model_display_name} · {formatDate(session.updated_at)}
                </span>
              </button>
            )}
            {renamingSessionId !== session.session_id ? (
              <span className="session-actions">
                <button
                  onClick={() => onStartRename(session)}
                  title={t("Rename conversation")}
                  aria-label={t("Rename conversation")}
                >
                  <Pencil size={13} />
                </button>
                <button
                  onClick={() => onDelete(session)}
                  title={t("Delete conversation")}
                  aria-label={t("Delete conversation")}
                >
                  <Trash2 size={13} />
                </button>
              </span>
            ) : null}
          </div>
        ))}
      </div>
      <div className="sidebar-footer">
        <span>{t("{{count}} conversations", { count: sessions.length })}</span>
        <span className="workspace-path" title={sessions[0]?.workspace_root}>
          {shortPath(sessions[0]?.workspace_root ?? "")}
        </span>
      </div>
    </aside>
  );
}
