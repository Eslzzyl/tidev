# TUI Diff Rendering Issues

## Current status

Two visual issues occurred in sequence:

1. The original issue was a jagged right edge on TUI diff cards.
2. After a transient background-rendering change removed that jagged edge, the
   user reported horizontally misaligned line numbers.

The user then requested reversion of every `crates/tidev-tui` change. Those
changes have been restored to the repository version. The root-edge fix and the
later line-number experiment are absent from the current TUI source. The
line-number cause remains unresolved.

## Original issue: jagged diff-card right edge

The initial report said that right-side alignment appeared to work only for
files with syntax highlighting. The user later clarified that the actual
symptom was the jagged right edge of the diff card, independent of file type.
Screenshots included `clipboard_3278x1798.png` during clarification.

### Diagnostic attempts

- The first explanations focused on line-number alignment, old/new row pairing,
  and a possible Unicode ambiguous-width difference involving `·` (U+00B7).
  The user rejected assumptions about terminal behavior and required a
  cross-platform, cross-language solution that preserves the original Unicode
  text.
- The proposed width-policy change was not the final implementation. The chosen
  direction was fixed Buffer-column geometry for card and diff backgrounds.

### Rendering attempts and observed results

1. **Fixed diff-fill ranges.** The renderer carried fixed left/right diff
   background ranges and painted them at Buffer columns. Formatting, the TUI
   tests (324), workspace check, Clippy, and `git diff --check` passed. The user
   reported that the card edge remained jagged; only the inner red/green fills
   had improved.
2. **Fixed full-card background range.** Inspection found that
   `decorate_card_lines` still derived the card-wide trailing fill from text
   widths. A full-card background range was added per row, followed by local
   span and diff backgrounds. The same checks passed. The user reported that
   the right-edge jaggedness was gone.

The second result was a user-observed visual improvement in the transient
working tree. The subsequent explicit reversion removed this implementation,
so the improvement is not present in the current TUI source.

## Subsequent issue: line-number alignment

Immediately after reporting the right edge fixed, the user reported line-number
misalignment and supplied `clipboard_302x1270.png`. A later attachment was
`clipboard_226x952.png`. In the visible image, line numbers around 274–280, and
some corresponding source text, appear shifted horizontally by about one
terminal cell on selected rows.

The screenshots establish the visual symptom, but the cause was not diagnosed.
The transcript does not determine whether the shift tracks wrapped lines, row
kind, a pane boundary, or another layout condition. The earlier claim that
wrapping caused it was a guess.

### Failed follow-up

A code change moved gutter insertion until after source-text wrapping and added
a Unicode wrapping test. `cargo test -p tidev-tui` passed with 325 tests and
Clippy passed. The user reported no visible change and asked for the edits to be
reverted. This experiment is reverted.

## Relevant renderer paths

In `crates/tidev-tui/src/diff_render.rs`:

- `render_diff_section` chooses narrow or side-by-side layout.
- `render_cell_lines` wraps each cell and adds its line-number gutter.
- `cell_prefix` and `blank_prefix` form first-line and continuation prefixes.
- `merge_columns` combines left and right cells in side-by-side layout.

The card decoration path is in
`crates/tidev-tui/src/components/chat/render/utils.rs`, including
`decorate_card_lines`. Buffer drawing and chat Paragraph rendering are in
`crates/tidev-tui/src/hyperlink.rs` and
`crates/tidev-tui/src/components/chat/render/mod.rs`.

## Requirements

- Preserve original Unicode text, including CJK, ambiguous-width characters,
  combining marks, emoji, and tabs after normal tab expansion.
- Derive card and diff background extents from fixed Buffer/layout columns.
- Avoid assumptions about external terminal width behavior.
- Keep the card-edge issue and line-number issue as distinct symptoms.

## Next investigation

1. Reproduce each screenshot symptom independently with Ratatui Buffer tests.
2. For the edge issue, assert the card's final background column and the red or
   green diff ranges on every rendered row.
3. For the line-number issue, assert Buffer columns for number digits, markers,
   source text, and wrapped continuations in narrow and wide layouts.
4. Compare those cell positions with the screenshots before changing layout
   code. Determine whether the line-number shift follows row kind, wrapping, or
   pane placement.
5. Keep unrelated working-tree changes untouched. Run `cargo fmt` and focused
   TUI tests for any future source change.
