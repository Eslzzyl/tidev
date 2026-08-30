import { Folder, Pencil, Plus, Search, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useWorkspace } from "../../hooks/workspaceQueries";
import type { Session } from "../../types/api";
import { formatDate, formatSessionActivity, shortPath } from "../../utils/chat";

export interface SessionSidebarProps {
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  mobileOpen?: boolean;
  sessions: Session[];
  workspaceRoots: string[];
  workspaceRootFilter: string | null;
  selectedSessionId: string | null;
  search: string;
  renamingSessionId: string | null;
  renameValue: string;
  onSearchChange: (value: string) => void;
  onWorkspaceRootFilterChange: (workspaceRoot: string | null) => void;
  onLoadMore: () => void;
  onCreate: () => void;
  onSelect: (sessionId: string) => void;
  onStartRename: (session: Session) => void;
  onRenameChange: (value: string) => void;
  onRename: (sessionId: string) => void;
  onCancelRename: () => void;
  onDelete: (session: Session) => void;
}

type SessionGroup = { label: string; sessions: Session[] };

function dateKey(value: string): string | null {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return [date.getFullYear(), date.getMonth(), date.getDate()].join("-");
}

function groupSessions(sessions: Session[], labels: Record<string, string>): SessionGroup[] {
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const sevenDaysAgo = new Date(today);
  sevenDaysAgo.setDate(sevenDaysAgo.getDate() - 6);
  const groups = new Map<string, Session[]>();

  for (const session of sessions) {
    const date = new Date(session.updated_at);
    const key = dateKey(session.updated_at);
    const group =
      !key || date >= today
        ? "today"
        : date >= yesterday
          ? "yesterday"
          : date >= sevenDaysAgo
            ? "previousWeek"
            : "older";
    groups.set(group, [...(groups.get(group) ?? []), session]);
  }

  return ["today", "yesterday", "previousWeek", "older"].flatMap((key) => {
    const grouped = groups.get(key);
    return grouped ? [{ label: labels[key], sessions: grouped }] : [];
  });
}

export function SessionSidebar({
  loading,
  loadingMore,
  hasMore,
  mobileOpen = false,
  sessions,
  workspaceRoots,
  workspaceRootFilter,
  selectedSessionId,
  search,
  renamingSessionId,
  renameValue,
  onSearchChange,
  onWorkspaceRootFilterChange,
  onLoadMore,
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

  const sessionGroups = useMemo(() => {
    if (search.trim()) return [{ label: t("Search results"), sessions }];
    return groupSessions(sessions, {
      today: t("Today"),
      yesterday: t("Yesterday"),
      previousWeek: t("Previous 7 days"),
      older: t("Older"),
    });
  }, [search, sessions, t]);

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

  const hasFilters = search.trim().length > 0 || workspaceRootFilter !== null;
  const workspaceDisplay = workspaceInfo?.workspace_display ?? "";
  const workspaceRoot = workspaceInfo?.workspace_root ?? "";

  return (
    <aside className={mobileOpen ? "session-sidebar mobile-open" : "session-sidebar"}>
      <div className="sidebar-heading">
        <strong>{t("Conversations")}</strong>
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
          <button
            className="icon-button"
            onClick={onCreate}
            title={t("New conversation")}
            aria-label={t("New conversation")}
          >
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
      <div className="session-filter">
        <label className="sr-only" htmlFor="session-workspace-filter">
          {t("Filter by directory")}
        </label>
        <Folder size={14} aria-hidden="true" />
        <select
          id="session-workspace-filter"
          value={workspaceRootFilter ?? ""}
          onChange={(event) => onWorkspaceRootFilterChange(event.target.value || null)}
        >
          <option value="">{t("All directories")}</option>
          {workspaceRoots.map((root) => (
            <option key={root} value={root}>
              {shortPath(root) || t("Unknown directory")}
            </option>
          ))}
        </select>
      </div>
      <div className="session-list">
        {loading && sessions.length === 0 ? (
          <div className="empty-state">{t("Loading sessions…")}</div>
        ) : null}
        {!loading && sessions.length === 0 ? (
          <div className="empty-state">
            {hasFilters ? t("No matching conversations.") : t("No conversations yet.")}
          </div>
        ) : null}
        {sessionGroups.map((group) => (
          <section className="session-group" key={group.label}>
            <h2>{group.label}</h2>
            {group.sessions.map((session) => (
              <div
                className={
                  selectedSessionId === session.session_id
                    ? "session-item selected"
                    : "session-item"
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
                    <span className="session-title">
                      {session.title || t("Untitled conversation")}
                    </span>
                    <span className="session-meta">
                      <span className="session-workspace" title={session.workspace_root}>
                        <Folder size={12} aria-hidden="true" />
                        {shortPath(session.workspace_root) || t("Unknown directory")}
                      </span>
                      <time className="session-date" title={formatDate(session.updated_at)}>
                        {formatSessionActivity(session.updated_at)}
                      </time>
                      {session.busy ? <span className="busy-indicator" /> : null}
                    </span>
                  </button>
                )}
                {renamingSessionId !== session.session_id ? (
                  <div className="session-actions">
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
                  </div>
                ) : null}
              </div>
            ))}
          </section>
        ))}
        {hasMore ? (
          <button className="session-load-more" onClick={onLoadMore} disabled={loadingMore}>
            {loadingMore ? t("Loading more conversations…") : t("Load more")}
          </button>
        ) : null}
      </div>
      {workspaceDisplay ? (
        <div className="sidebar-footer">
          <span className="workspace-path" title={workspaceRoot}>
            <Folder size={13} aria-hidden="true" />
            {workspaceDisplay}
          </span>
        </div>
      ) : null}
    </aside>
  );
}
