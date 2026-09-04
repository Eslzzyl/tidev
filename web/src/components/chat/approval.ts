import type { ApprovedTool, ToolCall, ToolCallWithViolations } from "../../types/api";

export interface QuestionOption {
  label: string;
  description?: string;
}

export interface QuestionInfo {
  question: string;
  header: string;
  options: QuestionOption[];
  multiple?: boolean;
  custom?: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function isQuestionTool(tool: ToolCall): boolean {
  return tool.name === "question";
}

export function parseQuestionArgs(argumentsJson: string): QuestionInfo[] | null {
  try {
    const value: unknown = JSON.parse(argumentsJson);
    if (!isRecord(value) || !Array.isArray(value.questions) || value.questions.length === 0) {
      return null;
    }

    const questions: QuestionInfo[] = [];
    for (const item of value.questions) {
      if (!isRecord(item) || typeof item.question !== "string" || typeof item.header !== "string") {
        return null;
      }
      if (!Array.isArray(item.options)) {
        return null;
      }

      const options: QuestionOption[] = [];
      for (const option of item.options) {
        if (!isRecord(option) || typeof option.label !== "string") {
          return null;
        }
        if (option.description !== undefined && typeof option.description !== "string") {
          return null;
        }
        options.push({
          label: option.label,
          ...(typeof option.description === "string" ? { description: option.description } : {}),
        });
      }

      if (item.multiple !== undefined && typeof item.multiple !== "boolean") {
        return null;
      }
      if (item.custom !== undefined && typeof item.custom !== "boolean") {
        return null;
      }

      questions.push({
        question: item.question,
        header: item.header,
        options,
        ...(typeof item.multiple === "boolean" ? { multiple: item.multiple } : {}),
        ...(typeof item.custom === "boolean" ? { custom: item.custom } : {}),
      });
    }
    return questions;
  } catch {
    return null;
  }
}

function toolResult(output: string) {
  return {
    output,
    attachments: [],
    metadata: {},
  };
}

export function rejectedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: toolResult("The user rejected this tool call."),
    child_session_id: null,
    allow_outside: false,
    sensitive_file_approved: false,
    user_reason: "Rejected in Web UI",
  };
}

export function approvedPermissionTool(item: ToolCallWithViolations): ApprovedTool {
  return {
    tool_call: item.tool_call,
    rejection: null,
    child_session_id: null,
    allow_outside: Boolean(item.workspace_boundary_violation),
    sensitive_file_approved: Boolean(item.sensitive_file_violation),
    user_reason: null,
  };
}

export function questionResult(tool: ToolCall, output: string | null): ApprovedTool {
  return {
    tool_call: tool,
    rejection: toolResult(output ?? "Tool 'question' was dismissed by user"),
    child_session_id: null,
    allow_outside: false,
    sensitive_file_approved: false,
    user_reason: null,
  };
}

export function invalidQuestionResult(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: toolResult("Tool 'question' was rejected: invalid or empty arguments."),
    child_session_id: null,
    allow_outside: false,
    sensitive_file_approved: false,
    user_reason: null,
  };
}

export function formatQuestionAnswers(questions: QuestionInfo[], answers: string[][]): string {
  return questions
    .flatMap((question, index) => {
      const selected = answers[index] ?? [];
      const value = selected.length === 0 ? "Unanswered" : selected.join(", ");
      return [`Q${index + 1}: ${question.question}`, `A: ${value}`];
    })
    .join("\n");
}
