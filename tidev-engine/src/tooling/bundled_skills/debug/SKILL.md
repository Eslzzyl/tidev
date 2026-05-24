---
name: debug
description: Systematic debugging methodology for any programming language or project. Use when investigating a bug, unexpected behavior, test failure, or error condition. Provides a structured approach to isolate root causes and verify fixes.
---

# Debugging Methodology

A structured approach to diagnosing and fixing bugs in any codebase.

## Core Process

Follow this cycle: **Reproduce → Isolate → Hypothesize → Verify → Fix**

### 1. Reproduce

- Get a reliable, minimal reproduction
- Note the exact input, expected output, and actual output
- Check if the bug is deterministic or intermittent
- If intermittent, look for timing, race conditions, or environmental factors

### 2. Isolate

Binary search to narrow down the root cause:

- **Recent changes**: What changed recently? Use `git bisect` or review recent commits
- **Divide code**: Comment out half the code, see if bug persists; repeat
- **Simplify input**: Reduce test input to minimal reproduction case
- **Check assumptions**: List every assumption the code makes and verify each
- **Add instrumentation**: Print/log key values at decision points

### 3. Hypothesize

Form a theory about the root cause:

- "The bug is caused by X because Y"
- Make predictions: "If I change X, the bug will/won't appear"
- Test your hypothesis with a targeted experiment
- If wrong, refine or discard the hypothesis

### 4. Verify

- Write a test that fails with the bug and passes once fixed
- Confirm the fix doesn't break related functionality
- Test edge cases around the fix
- Run the full test suite to check for regressions

### 5. Fix

- Make the smallest correct change
- Add a regression test
- Document what caused the bug and why the fix works

## Common Bug Patterns

Check these categories when you're stuck:

**Data & State**
- Uninitialized or stale variables
- Mutation when copy was expected
- Incorrect default values
- Cached data that should be invalidated

**Control Flow**
- Wrong branch taken (off-by-one, inverted condition)
- Missing early return / break / continue
- Exception/error swallowed silently
- Async/sync mismatch

**Input & Output**
- Encoding issues (UTF-8, line endings)
- Truncated data (buffer limits, field sizes)
- Format mismatch (JSON schema, CSV columns)
- Timezone/datetime handling

**Concurrency**
- Race condition (non-atomic read-modify-write)
- Deadlock (circular lock ordering)
- Starvation (one task hogs resource)
- Missing synchronization

**External Dependencies**
- API contract changed
- Network timeout / retry logic wrong
- File system permissions
- Environment-dependent behavior

## When to Use Sub-Agents

- **@explorer**: Search for related code, find patterns, discover how things are wired
- **@oracle**: Strategic debugging advice, architecture analysis, complex root cause analysis
- **@librarian**: Look up API docs, library behavior, specification details

## Documentation

After fixing, store what you learned:
- Record the root cause and fix in memory for future reference
- Add regression test comments explaining what was caught
- Consider updating AGENTS.md if the bug reveals a project-specific gotcha
