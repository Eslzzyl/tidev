import { Pencil, Plus, Search, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useWorkspace } from "../../hooks/workspaceQueries";
import type { Session } from "../../types/api";
import { formatDate, shortPath } from "../../utils/chat";

export interface SessionSidebarProps {
  loading: boolean;
  mobileOpen?: boolean;
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
  mobileOpen = false,
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
  const { data: workspaceInfo } = useWorkspace();
  const [searchOpen, setSearchOpen] = useState(() => search.trim().length > 0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const normalizedSearch = search.trim().toLowerCase();
  const visibleSessions = sessions.filter((session) =>
    session.title.toLowerCase().includes(normalizedSearch),
  );
  const selectedWorkspaceRoot = sessions.find(
    (session) => session.session_id === selectedSessionId,
  )?.workspace_root;
  const workspaceRoot = workspaceInfo?.workspace_root ?? selectedWorkspaceRoot;
  const workspaceDisplay =
    workspaceInfo?.workspace_display ?? (workspaceRoot ? shortPath(workspaceRoot) : "");

  useEffect(() => {
    if (searchOpen) searchInputRef.current?.focus();
  }, [searchOpen]);

  const handleSearchToggle = () => {
    if (searchOpen) onSearchChange("");
    setSearchOpen((open) => !open);
  };

  const handleSearchClose = () => {
    onSearchChange("");
    setSearchOpen(false);
  };

  return (
    <aside className={mobileOpen ? "session-sidebar mobile-open" : "session-sidebar"}>
      <div className="sidebar-heading">
        <div>
          <span className="eyebrow">{t("Workspace")}</span>
          <strong>{t("Conversations")}</strong>
        </div>
        <div className="sidebar-actions">
          <button
            className={searchOpen ? "icon-button active" : "icon-button"}
            onClick={handleSearchToggle}
            title={searchOpen ? t("Close search") : t("Search sessions")}
            aria-label={searchOpen ? t("Close search") : t("Search sessions")}
            aria-expanded={searchOpen}
          >
            <Search size={16} />
          </button>
          <button className="icon-button" onClick={onCreate} title={t("New conversation")}>
            <Plus size={17} />
          </button>
        </div>
      </div>
      <div
        className={searchOpen ? "session-search expanded" : "session-search collapsed"}
        aria-hidden={!searchOpen}
      >
        <Search size={14} />
        <input
          ref={searchInputRef}
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") handleSearchClose();
          }}
          placeholder={t("Search sessions…")}
          aria-label={t("Search sessions")}
          tabIndex={searchOpen ? 0 : -1}
        />
        {search ? (
          <button
            className="session-search-clear"
            onClick={() => {
              onSearchChange("");
              searchInputRef.current?.focus();
            }}
            title={t("Clear search")}
            aria-label={t("Clear search")}
            tabIndex={searchOpen ? 0 : -1}
          >
            <X size={14} />
          </button>
        ) : null}
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
                  <span className="session-model">{session.model_display_name}</span>
                  <span className="session-meta-separator" aria-hidden="true">
                    ·
                  </span>
                  <time className="session-date">{formatDate(session.updated_at)}</time>
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
      {workspaceDisplay ? (
        <div className="sidebar-footer">
          <span className="workspace-path" title={workspaceRoot}>
            {workspaceDisplay}
          </span>
        </div>
      ) : null}
    </aside>
  );
}
