import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

/**
 * Create a CodeMirror theme extension for the given color scheme.
 * @param dark Whether the theme is dark mode.
 */
export function createCodeMirrorTheme(dark: boolean) {
  const bg = dark ? "#1a1a2e" : "#ffffff";
  const bgGutter = dark ? "#16162a" : "#f5f5f5";
  const text = dark ? "#e0e0e0" : "#333333";
  const gutterText = dark ? "#6b6b8a" : "#999999";
  const cursor = dark ? "#c9c9ff" : "#333333";
  const selection = dark
    ? "rgba(100, 100, 200, 0.3)"
    : "rgba(0, 100, 200, 0.15)";
  const selectionMatch = dark
    ? "rgba(100, 100, 200, 0.2)"
    : "rgba(0, 100, 200, 0.1)";
  const lineHighlight = dark
    ? "rgba(255, 255, 255, 0.04)"
    : "rgba(0, 0, 0, 0.04)";
  const activeLineGutter = dark
    ? "rgba(255, 255, 255, 0.08)"
    : "rgba(0, 0, 0, 0.07)";
  const border = dark ? "#2a2a3e" : "#e5e5e5";
  const searchMatch = dark
    ? "rgba(200, 180, 80, 0.4)"
    : "rgba(255, 200, 0, 0.3)";
  const searchSel = dark ? "rgba(200, 180, 80, 0.6)" : "rgba(255, 200, 0, 0.5)";
  const tooltipBg = dark ? "#2a2a3e" : "#ffffff";
  const tooltipBorder = dark ? "#3a3a4e" : "#dddddd";
  const foldPlaceholder = dark ? "#3a3a4e" : "#eeeeee";

  return [
    EditorView.theme(
      {
        "&": {
          backgroundColor: bg,
          color: text,
          height: "100%",
          fontSize: "13px",
          fontFamily:
            "'JetBrains Mono', 'Fira Code', 'Cascadia Code', 'Consolas', monospace",
        },
        ".cm-scroller": {
          fontFamily: "inherit",
          lineHeight: "1.6",
        },
        ".cm-gutters": {
          backgroundColor: bgGutter,
          color: gutterText,
          borderRight: `1px solid ${border}`,
          userSelect: "none",
        },
        ".cm-gutterElement": {
          paddingLeft: "12px",
          paddingRight: "8px",
        },
        ".cm-activeLineGutter": {
          backgroundColor: activeLineGutter,
        },
        ".cm-activeLine": {
          backgroundColor: lineHighlight,
        },
        ".cm-cursor": {
          borderLeftColor: cursor,
        },
        ".cm-selectionBackground": {
          backgroundColor: selection,
        },
        "&.cm-focused .cm-selectionBackground": {
          backgroundColor: selection,
        },
        ".cm-selectionMatch": {
          backgroundColor: selectionMatch,
        },
        ".cm-matchingBracket": {
          backgroundColor: dark
            ? "rgba(100, 200, 100, 0.2)"
            : "rgba(0, 150, 0, 0.15)",
          outline: `1px solid ${dark ? "rgba(100, 200, 100, 0.3)" : "rgba(0, 150, 0, 0.3)"}`,
        },
        ".cm-nonmatchingBracket": {
          backgroundColor: dark
            ? "rgba(200, 100, 100, 0.2)"
            : "rgba(200, 0, 0, 0.15)",
        },
        ".cm-searchMatch": {
          backgroundColor: searchMatch,
          outline: `1px solid ${searchSel}`,
        },
        ".cm-searchMatch.cm-searchMatch-selected": {
          backgroundColor: searchSel,
        },
        ".cm-foldPlaceholder": {
          backgroundColor: foldPlaceholder,
          border: "none",
          borderRadius: "3px",
          fontFamily: "inherit",
        },
        ".cm-tooltip": {
          backgroundColor: tooltipBg,
          border: `1px solid ${tooltipBorder}`,
          borderRadius: "4px",
          boxShadow: "0 2px 6px rgba(0,0,0,0.15)",
        },
        ".cm-tooltip-autocomplete": {
          "& > ul": {
            fontFamily: "inherit",
            fontSize: "12px",
            maxHeight: "200px",
          },
          "& > ul > li": {
            padding: "2px 8px",
          },
          "& > ul > li[aria-selected]": {
            backgroundColor: dark
              ? "rgba(100, 100, 200, 0.3)"
              : "rgba(0, 100, 200, 0.1)",
          },
        },
        ".cm-completionLabel": {
          fontFamily: "inherit",
        },
        ".cm-completionDetail": {
          fontFamily: "inherit",
          color: dark ? "#888" : "#999",
        },
        ".cm-line": {
          paddingLeft: "4px",
        },
        "&.cm-focused": {
          outline: "none",
        },
        ".cm-selectionLayer": {
          zIndex: 0,
        },
      },
      { dark },
    ),
    syntaxHighlighting(
      HighlightStyle.define([
        { tag: t.keyword, color: dark ? "#c678dd" : "#d73a49" },
        { tag: t.atom, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.definition(t.name), color: dark ? "#e5c07b" : "#953800" },
        { tag: t.operator, color: dark ? "#abb2bf" : "#0550ae" },
        { tag: t.propertyName, color: dark ? "#e06c75" : "#0550ae" },
        {
          tag: t.comment,
          color: dark ? "#5c6370" : "#6e7781",
          fontStyle: "italic",
        },
        { tag: t.string, color: dark ? "#98c379" : "#0a3069" },
        { tag: t.number, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.regexp, color: dark ? "#98c379" : "#0a3069" },
        { tag: t.variableName, color: dark ? "#e06c75" : "#953800" },
        { tag: t.punctuation, color: dark ? "#abb2bf" : "#333333" },
        {
          tag: t.heading,
          color: dark ? "#61afef" : "#0550ae",
          fontWeight: "bold",
        },
        {
          tag: t.emphasis,
          color: dark ? "#e06c75" : "#333333",
          fontStyle: "italic",
        },
        {
          tag: t.link,
          color: dark ? "#61afef" : "#0969da",
          textDecoration: "underline",
        },
        { tag: t.list, color: dark ? "#98c379" : "#0550ae" },
        { tag: t.typeName, color: dark ? "#e5c07b" : "#0550ae" },
        { tag: t.className, color: dark ? "#e5c07b" : "#0550ae" },
        { tag: t.tagName, color: dark ? "#e06c75" : "#116329" },
        { tag: t.attributeName, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.attributeValue, color: dark ? "#98c379" : "#0a3069" },
        { tag: t.meta, color: dark ? "#61afef" : "#333333" },
        { tag: t.deleted, color: dark ? "#e06c75" : "#82071e" },
        { tag: t.inserted, color: dark ? "#98c379" : "#116329" },
        { tag: t.changed, color: dark ? "#d19a66" : "#953800" },
        { tag: t.literal, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.bool, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.moduleKeyword, color: dark ? "#c678dd" : "#d73a49" },
        { tag: t.self, color: dark ? "#e06c75" : "#953800" },
        { tag: t.separator, color: dark ? "#abb2bf" : "#333333" },
        { tag: t.bracket, color: dark ? "#abb2bf" : "#333333" },
        { tag: t.angleBracket, color: dark ? "#abb2bf" : "#333333" },
        { tag: t.squareBracket, color: dark ? "#abb2bf" : "#333333" },
        { tag: t.paren, color: dark ? "#abb2bf" : "#333333" },
        { tag: t.brace, color: dark ? "#abb2bf" : "#333333" },
        { tag: t.derefOperator, color: dark ? "#61afef" : "#0550ae" },
        { tag: t.arithmeticOperator, color: dark ? "#61afef" : "#0550ae" },
        { tag: t.logicOperator, color: dark ? "#61afef" : "#0550ae" },
        { tag: t.compareOperator, color: dark ? "#61afef" : "#0550ae" },
        { tag: t.updateOperator, color: dark ? "#61afef" : "#0550ae" },
        { tag: t.definitionOperator, color: dark ? "#61afef" : "#0550ae" },
        { tag: t.controlKeyword, color: dark ? "#c678dd" : "#d73a49" },
        { tag: t.definitionKeyword, color: dark ? "#c678dd" : "#d73a49" },
        { tag: t.modifier, color: dark ? "#c678dd" : "#d73a49" },
        { tag: t.labelName, color: dark ? "#e06c75" : "#953800" },
        { tag: t.namespace, color: dark ? "#e5c07b" : "#0550ae" },
        {
          tag: t.lineComment,
          color: dark ? "#5c6370" : "#6e7781",
          fontStyle: "italic",
        },
        {
          tag: t.blockComment,
          color: dark ? "#5c6370" : "#6e7781",
          fontStyle: "italic",
        },
        { tag: t.url, color: dark ? "#61afef" : "#0969da" },
        { tag: t.invalid, color: dark ? "#e06c75" : "#82071e" },
        { tag: t.character, color: dark ? "#98c379" : "#0a3069" },
        { tag: t.escape, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.color, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.content, color: dark ? "#e0e0e0" : "#333333" },
        { tag: t.monospace, color: dark ? "#e0e0e0" : "#333333" },
        {
          tag: t.strong,
          color: dark ? "#e0e0e0" : "#333333",
          fontWeight: "bold",
        },
        { tag: t.strikethrough, color: dark ? "#e0e0e0" : "#333333" },
        { tag: t.quote, color: dark ? "#98c379" : "#0a3069" },
        { tag: t.unit, color: dark ? "#d19a66" : "#0550ae" },
        { tag: t.null, color: dark ? "#d19a66" : "#0550ae" },
      ]),
    ),
  ];
}
