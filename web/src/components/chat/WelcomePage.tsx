import { useRef, useState } from "react";
import { Clock3, ChevronDown, LoaderCircle, Send } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { Model, Session } from "../../types/api";
import { formatDate, formatThinkingLevel } from "../../utils/chat";

export interface WelcomePageProps {
  draft: string;
  error: string | null;
  loading: boolean;
  mode: "build" | "plan";
  enterToSend: boolean;
  sending: boolean;
  sessions: Session[];
  models: Model[];
  activeModel: Model | undefined;
  thinkingLevel: string | undefined;
  onChangeDraft: (value: string) => void;
  onModeChange: (mode: "build" | "plan") => void;
  onSelectSession: (sessionId: string) => void;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onSubmit: () => void;
}

export function WelcomePage({
  draft,
  error,
  loading,
  mode,
  enterToSend,
  sending,
  sessions,
  models,
  activeModel,
  thinkingLevel,
  onChangeDraft,
  onModeChange,
  onSelectSession,
  onSelectModel,
  onSelectThinkingLevel,
  onSubmit,
}: WelcomePageProps) {
  const { t } = useTranslation();
  const compositionRef = useRef(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [thinkingOpen, setThinkingOpen] = useState(false);

  return (
    <section className="welcome-page">
      <div className="welcome-heading">
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>{t("Your intelligent coding assistant")}</p>
      </div>
      <div className="welcome-composer">
        <textarea
          value={draft}
          onChange={(event) => onChangeDraft(event.target.value)}
          onCompositionStart={() => {
            compositionRef.current = true;
          }}
          onCompositionEnd={() => {
            compositionRef.current = false;
          }}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.nativeEvent.isComposing &&
              !compositionRef.current &&
              ((enterToSend && !event.shiftKey) ||
                (!enterToSend && (event.ctrlKey || event.metaKey)))
            ) {
              event.preventDefault();
              onSubmit();
            }
          }}
          autoFocus
          disabled={loading || sending}
          placeholder={t("What would you like to work on?")}
          rows={3}
        />
        <div className="welcome-composer-footer">
          <div className="welcome-controls">
            <button
              className={mode === "plan" ? "composer-control plan" : "composer-control build"}
              onClick={() => onModeChange(mode === "plan" ? "build" : "plan")}
            >
              {mode === "plan" ? t("Plan") : t("Build")}
            </button>
            <div className="composer-menu">
              <button
                className="composer-control neutral"
                onClick={() => {
                  setModelOpen((current) => !current);
                  setThinkingOpen(false);
                }}
              >
                <span>
                  {activeModel
                    ? `${activeModel.provider_display_name}/${activeModel.model_display_name}`
                    : t("Select model")}
                </span>
                <ChevronDown size={13} />
              </button>
              {modelOpen ? (
                <div className="composer-popover model-popover">
                  {models.map((model) => (
                    <button
                      className={model.active ? "composer-option selected" : "composer-option"}
                      disabled={!model.connected}
                      key={`${model.provider_id}:${model.model_id}`}
                      onClick={() => {
                        onSelectModel(model);
                        setModelOpen(false);
                      }}
                    >
                      <span>
                        {model.provider_display_name}/{model.model_display_name}
                      </span>
                      <small>{model.connected ? t("Connected") : t("Not connected")}</small>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
            {activeModel?.thinking_levels.length ? (
              <div className="composer-menu">
                <button
                  className="composer-control thinking"
                  onClick={() => {
                    setThinkingOpen((current) => !current);
                    setModelOpen(false);
                  }}
                >
                  <span>{formatThinkingLevel(thinkingLevel ?? activeModel.thinking_level)}</span>
                  <ChevronDown size={13} />
                </button>
                {thinkingOpen ? (
                  <div className="composer-popover thinking-popover">
                    {activeModel.thinking_levels.map((level) => (
                      <button
                        className={
                          thinkingLevel === level ? "composer-option selected" : "composer-option"
                        }
                        key={level}
                        onClick={() => {
                          onSelectThinkingLevel(level);
                          setThinkingOpen(false);
                        }}
                      >
                        {formatThinkingLevel(level)}
                      </button>
                    ))}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
          <button
            className="send-button"
            disabled={!draft.trim() || loading || sending}
            onClick={onSubmit}
            title={t("Start conversation")}
          >
            {sending ? <LoaderCircle className="spin" size={17} /> : <Send size={17} />}
          </button>
        </div>
      </div>
      {error ? <div className="error-banner welcome-error">{error}</div> : null}
      {sessions.length > 0 ? (
        <div className="recent-sessions">
          <div className="recent-heading">
            <Clock3 size={16} />
            <span>{t("Recent Sessions")}</span>
          </div>
          <div className="recent-session-grid">
            {sessions.slice(0, 5).map((session) => (
              <button
                className="recent-session"
                key={session.session_id}
                onClick={() => onSelectSession(session.session_id)}
              >
                <span>{session.title || t("Untitled conversation")}</span>
                <time>{formatDate(session.updated_at)}</time>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
}
