import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ApprovedTool, FrontendRequest, ProviderErrorData, ToolCall } from "../../types/api";
import type { StreamMessage } from "../../types/chat";
import { useTranslation } from "react-i18next";
import { Check, CircleAlert, ChevronRight, LoaderCircle, Pencil, RefreshCw, X } from "lucide-react";

import {
  approvedPermissionTool,
  formatQuestionAnswers,
  invalidQuestionResult,
  isQuestionTool,
  parseQuestionArgs,
  questionResult,
  rejectedTool,
} from "./approval";
import { Button, IconButton, Textarea } from "../ui";

export function ProviderErrorCard({
  error,
  messageId,
  retrying,
  onRetry,
}: {
  error: ProviderErrorData;
  messageId: string;
  retrying?: StreamMessage["retrying"];
  onRetry?: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const canRetry = error.retryable && !retrying && Boolean(messageId) && Boolean(onRetry);
  const className = [
    "chat-message",
    "assistant-message",
    "assistant-segment-row",
    "provider-error-row",
    retrying ? "is-retrying" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <article className={className}>
      <div
        className={retrying ? "provider-error-card is-retrying" : "provider-error-card"}
        role="alert"
        aria-live={retrying ? "polite" : "assertive"}
      >
        <span className={retrying ? "provider-error-icon is-retrying" : "provider-error-icon"}>
          {retrying ? <LoaderCircle className="spin" size={21} /> : <CircleAlert size={21} />}
        </span>
        <div className="provider-error-content">
          <p className="provider-error-message">{error.message}</p>
          {retrying ? (
            <span className="provider-error-meta">
              {t("Retrying {{attempt}}/{{maxAttempts}}", {
                attempt: retrying.attempt,
                maxAttempts: retrying.maxAttempts,
              })}
            </span>
          ) : null}
        </div>
        {canRetry ? (
          <Button
            type="button"
            className="provider-error-retry"
            onClick={() => onRetry?.(messageId)}
            variant="secondary"
            size="sm"
            leadingIcon={<RefreshCw size={15} />}
          >
            {t("Retry")}
          </Button>
        ) : null}
      </div>
    </article>
  );
}

function QuestionApproval({
  tool,
  onDecide,
}: {
  tool: ToolCall;
  onDecide: (decision: ApprovedTool) => void;
}) {
  const { t } = useTranslation();
  const questions = useMemo(() => parseQuestionArgs(tool.arguments), [tool.arguments]);
  const [questionIndex, setQuestionIndex] = useState(0);
  const [answers, setAnswers] = useState<string[][]>(() => questions?.map(() => []) ?? []);
  const [customAnswer, setCustomAnswer] = useState("");
  const [customAnswerOpen, setCustomAnswerOpen] = useState(false);
  const customTextareaRef = useRef<HTMLTextAreaElement>(null);
  const customComposingRef = useRef(false);
  const customCompositionJustCommittedRef = useRef(false);
  const customCompositionEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setQuestionIndex(0);
    setAnswers(questions?.map(() => []) ?? []);
    setCustomAnswer("");
    setCustomAnswerOpen(false);
  }, [questions]);

  useEffect(() => {
    if (!customAnswerOpen) return;
    const textarea = customTextareaRef.current;
    if (!textarea) return;

    textarea.style.height = "auto";
    const height = Math.min(textarea.scrollHeight, 200);
    textarea.style.height = `${height}px`;
    textarea.style.overflowY = textarea.scrollHeight > 200 ? "auto" : "hidden";
  }, [customAnswer, customAnswerOpen]);

  useEffect(
    () => () => {
      if (customCompositionEndTimerRef.current) {
        clearTimeout(customCompositionEndTimerRef.current);
      }
    },
    [],
  );

  if (!questions) {
    return (
      <div className="question-composer question-composer-invalid" role="alert">
        <div className="question-composer-header">
          <p>{t("This question request has invalid or empty arguments.")}</p>
          <IconButton
            className="question-composer-dismiss"
            label={t("Dismiss question")}
            onClick={() => onDecide(invalidQuestionResult(tool))}
            size="sm"
            variant="ghost"
          >
            <X size={16} />
          </IconButton>
        </div>
      </div>
    );
  }

  const question = questions[questionIndex];
  const multiple = question.multiple ?? false;
  const selectedAnswers = answers[questionIndex] ?? [];
  const allowCustom = question.custom ?? true;

  const advance = (nextAnswers: string[][]) => {
    if (questionIndex + 1 >= questions.length) {
      onDecide(questionResult(tool, formatQuestionAnswers(questions, nextAnswers)));
      return;
    }
    setAnswers(nextAnswers);
    setQuestionIndex((index) => index + 1);
    setCustomAnswer("");
    setCustomAnswerOpen(false);
  };

  const setCurrentAnswers = (nextSelected: string[]) =>
    answers.map((selected, index) => (index === questionIndex ? nextSelected : selected));

  const selectOption = (label: string) => {
    if (!multiple) {
      advance(setCurrentAnswers([label]));
      return;
    }

    setAnswers(
      setCurrentAnswers(
        selectedAnswers.includes(label)
          ? selectedAnswers.filter((value) => value !== label)
          : [...selectedAnswers, label],
      ),
    );
  };

  const continueWithCurrentAnswer = () => {
    const custom = customAnswer.trim();
    const nextSelected = custom
      ? multiple
        ? selectedAnswers.includes(custom)
          ? selectedAnswers
          : [...selectedAnswers, custom]
        : [custom]
      : selectedAnswers;
    if (nextSelected.length === 0) return;
    advance(setCurrentAnswers(nextSelected));
  };

  return (
    <div
      className="question-composer"
      role="group"
      aria-label={question.header || question.question}
    >
      <div className="question-composer-header">
        <div>
          <p>{question.question}</p>
          {questions.length > 1 ? (
            <span>
              {t("Question {{current}} of {{total}}", {
                current: questionIndex + 1,
                total: questions.length,
              })}
            </span>
          ) : null}
        </div>
      </div>
      {question.options.length > 0 ? (
        <div className="question-composer-options">
          {question.options.map((option, optionIndex) => {
            const selected = selectedAnswers.includes(option.label);
            return (
              <button
                aria-pressed={multiple ? selected : undefined}
                className={
                  selected ? "question-composer-option selected" : "question-composer-option"
                }
                key={`${tool.id}-${questionIndex}-${optionIndex}`}
                onClick={() => selectOption(option.label)}
                type="button"
              >
                <span className="question-composer-option-index">{optionIndex + 1}</span>
                <span className="question-composer-option-copy">
                  <strong>{option.label}</strong>
                  {option.description ? <small>{option.description}</small> : null}
                </span>
                {multiple ? (
                  selected ? (
                    <Check className="question-composer-option-check" size={17} />
                  ) : null
                ) : (
                  <ChevronRight className="question-composer-option-arrow" size={18} />
                )}
              </button>
            );
          })}
        </div>
      ) : null}
      {allowCustom ? (
        <div className="question-composer-custom">
          {customAnswerOpen ? (
            <div className="question-composer-custom-editor">
              <span className="question-composer-custom-leading" aria-hidden="true">
                <Pencil size={15} />
              </span>
              <Textarea
                autoFocus
                ref={customTextareaRef}
                onChange={(event) => setCustomAnswer(event.target.value)}
                onCompositionStart={() => {
                  customComposingRef.current = true;
                  customCompositionJustCommittedRef.current = false;
                  if (customCompositionEndTimerRef.current) {
                    clearTimeout(customCompositionEndTimerRef.current);
                  }
                }}
                onCompositionEnd={() => {
                  customComposingRef.current = false;
                  customCompositionJustCommittedRef.current = true;
                  if (customCompositionEndTimerRef.current) {
                    clearTimeout(customCompositionEndTimerRef.current);
                  }
                  customCompositionEndTimerRef.current = setTimeout(() => {
                    customCompositionJustCommittedRef.current = false;
                  }, 0);
                }}
                onKeyDown={(event) => {
                  if (
                    event.key === "Enter" &&
                    !event.shiftKey &&
                    !event.nativeEvent.isComposing &&
                    !customComposingRef.current &&
                    !customCompositionJustCommittedRef.current
                  ) {
                    event.preventDefault();
                    continueWithCurrentAnswer();
                  }
                }}
                placeholder={t("Type a custom answer")}
                rows={1}
                value={customAnswer}
              />
            </div>
          ) : (
            <button
              className="question-composer-custom-trigger"
              onClick={() => setCustomAnswerOpen(true)}
              type="button"
            >
              <span className="question-composer-custom-leading" aria-hidden="true">
                <Pencil size={15} />
              </span>
              <span>{t("Type a custom answer")}</span>
            </button>
          )}
        </div>
      ) : null}
      <div className="question-composer-footer">
        <Button
          className="question-composer-skip"
          type="button"
          onClick={() => onDecide(questionResult(tool, null))}
          variant="secondary"
          size="sm"
        >
          {t("Skip")}
        </Button>
        {multiple || customAnswer.trim() ? (
          <Button
            className="question-composer-continue"
            disabled={selectedAnswers.length === 0 && !customAnswer.trim()}
            onClick={continueWithCurrentAnswer}
            size="sm"
            variant="primary"
          >
            {t("Continue")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export function ApprovalCard({
  request,
  onRespond,
}: {
  request: FrontendRequest;
  onRespond: (tools: ApprovedTool[]) => void;
}) {
  const { t } = useTranslation();
  const tools = request.kind.ToolApproval ?? [];
  const [toolIndex, setToolIndex] = useState(0);
  const [decisions, setDecisions] = useState<ApprovedTool[]>([]);
  const current = tools[toolIndex];

  useEffect(() => {
    setToolIndex(0);
    setDecisions([]);
  }, [request.request_id]);

  const decide = useCallback(
    (decision: ApprovedTool) => {
      const next = [...decisions, decision];
      if (toolIndex + 1 >= tools.length) {
        onRespond(next);
        return;
      }
      setDecisions(next);
      setToolIndex((index) => index + 1);
    },
    [decisions, onRespond, toolIndex, tools.length],
  );

  if (!current) {
    return null;
  }

  const isQuestion = isQuestionTool(current.tool_call);
  if (isQuestion) {
    return <QuestionApproval tool={current.tool_call} onDecide={decide} />;
  }

  return (
    <div className="permission-composer">
      <div className="permission-composer-header">
        <p>{t("Allow tidev to run {{toolName}}?", { toolName: current.tool_call.name })}</p>
      </div>
      {tools.length > 1 ? (
        <span className="permission-composer-progress">
          {t("Step {{current}} / {{total}}", { current: toolIndex + 1, total: tools.length })}
        </span>
      ) : null}
      <div className="permission-composer-tool">
        <code>{current.tool_call.arguments}</code>
      </div>
      <div className="permission-composer-risks">
        {current.workspace_boundary_violation ? (
          <div className="permission-composer-risk">
            <CircleAlert size={16} aria-hidden="true" />
            <div>
              <strong>{t("Workspace boundary access")}</strong>
              <code>{current.workspace_boundary_violation}</code>
            </div>
          </div>
        ) : null}
        {current.sensitive_file_violation ? (
          <div className="permission-composer-risk">
            <CircleAlert size={16} aria-hidden="true" />
            <div>
              <strong>{t("Sensitive file access")}</strong>
              <code>{current.sensitive_file_violation}</code>
            </div>
          </div>
        ) : null}
      </div>
      <div className="permission-composer-footer">
        <Button
          className="permission-composer-reject"
          type="button"
          onClick={() => decide(rejectedTool(current.tool_call))}
          variant="secondary"
          size="sm"
          leadingIcon={<X size={15} />}
        >
          {t("Reject")}
        </Button>
        <Button
          className="permission-composer-allow"
          type="button"
          onClick={() => decide(approvedPermissionTool(current))}
          variant="primary"
          size="sm"
          leadingIcon={<Check size={15} />}
        >
          {t("Allow")}
        </Button>
      </div>
    </div>
  );
}
