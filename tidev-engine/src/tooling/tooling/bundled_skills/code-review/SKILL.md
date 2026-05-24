---
name: code-review
description: General code review methodology covering logic, security, performance, readability, testing, and breaking changes. Use when asked to review code, check a pull request, evaluate a diff, or assess code quality in any language.
---

# Code Review

Systematic methodology for reviewing code changes.

## Review Checklist

Go through these dimensions in order. Do not stop after finding one issue — analyze the change comprehensively.

### 1. Correctness & Logic

- Does the code do what it claims to do?
- Are edge cases handled (empty input, null/nil, boundary values)?
- Are there off-by-one errors, race conditions, or integer overflows?
- Is error handling present and appropriate?
- Do async/concurrent operations have proper synchronization?

### 2. Security

- Are user inputs validated and sanitized?
- Are there hardcoded secrets, tokens, or credentials?
- Is authentication/authorization checked at the right layer?
- Are file paths, URLs, or shell commands constructed safely (no injection)?
- Are dependencies up-to-date without known vulnerabilities?

### 3. Performance

- Are there obvious inefficiencies (N+1 queries, redundant work, large allocations)?
- Is I/O batched where possible?
- Are hot paths reasonably optimized?
- Would this change scale acceptably?

### 4. Readability & Maintainability

- Are names clear and descriptive?
- Is the code structured logically (functions/methods do one thing)?
- Are there unnecessary comments or missing important ones?
- Is the change as small as possible while still being correct?
- Would another developer understand this code without context?

### 5. Testing

- Are there tests for the new/changed code?
- Do tests cover normal cases, edge cases, and error conditions?
- Are tests deterministic and fast?
- Is test coverage appropriate for the risk level of the change?

### 6. Breaking Changes

- Does the change modify any public API, interface, or contract?
- Does it change configuration format, CLI arguments, or environment variables?
- Does it alter database schema, file formats, or wire protocols?
- Does it remove or rename previously exposed functionality?
- If yes, is there a migration path or deprecation strategy?

## Providing Feedback

- **Be specific**: cite exact lines and explain *why* something is a problem
- **Suggest alternatives**: don't just point out issues, propose fixes
- **Prioritize**: separate "must fix" (correctness, security) from "nice to have" (style)
- **Ask questions**: if something is unclear, ask rather than assume
- **Acknowledge good parts**: positive feedback helps reinforce good practices

## Scope Assessment

For each change, evaluate:
- **Size**: Is the diff manageable? If >500 lines of non-mechanical change, can it be split?
- **Impact**: Does this affect other parts of the system?
- **Risk**: How likely is this to introduce regressions?
