import { describe, expect, it } from "vitest";

import {
  approvedPermissionTool,
  formatQuestionAnswers,
  parseQuestionArgs,
  questionResult,
} from "./approval";

const questionTool = {
  id: "question-1",
  name: "question",
  arguments:
    '{"questions":[{"question":"Choose a color","header":"Color","options":[{"label":"Blue"},{"label":"Green"}]},{"question":"Explain why","header":"Reason","options":[],"custom":true}]}',
};

describe("approval helpers", () => {
  it("parses and formats question answers using the TUI result contract", () => {
    const questions = parseQuestionArgs(questionTool.arguments);
    expect(questions).not.toBeNull();
    expect(formatQuestionAnswers(questions!, [["Blue"], ["Because it is calm"]])).toBe(
      "Q1: Choose a color\nA: Blue\nQ2: Explain why\nA: Because it is calm",
    );

    const response = questionResult(questionTool, "Q1: Choose a color\nA: Blue");
    expect(response.rejection?.output).toBe("Q1: Choose a color\nA: Blue");
    expect(response.allow_outside).toBe(false);
    expect(response.sensitive_file_approved).toBe(false);
  });

  it("rejects malformed question arguments", () => {
    expect(parseQuestionArgs('{"questions":[{"question":"Missing options"}]}')).toBeNull();
    expect(parseQuestionArgs('{"questions":[]}')).toBeNull();
  });

  it("grants only the permission represented by the violation", () => {
    const permission = approvedPermissionTool({
      tool_call: questionTool,
      workspace_boundary_violation: "C:/outside.txt",
      sensitive_file_violation: null,
    });
    expect(permission.allow_outside).toBe(true);
    expect(permission.sensitive_file_approved).toBe(false);
  });
});
