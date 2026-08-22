import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import dart from "highlight.js/lib/languages/dart";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import less from "highlight.js/lib/languages/less";
import lua from "highlight.js/lib/languages/lua";
import markdown from "highlight.js/lib/languages/markdown";
import nim from "highlight.js/lib/languages/nim";
import php from "highlight.js/lib/languages/php";
import python from "highlight.js/lib/languages/python";
import r from "highlight.js/lib/languages/r";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import scala from "highlight.js/lib/languages/scala";
import scss from "highlight.js/lib/languages/scss";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const languages = {
  bash,
  c,
  cpp,
  csharp,
  css,
  dart,
  go,
  java,
  javascript,
  json,
  kotlin,
  less,
  lua,
  markdown,
  nim,
  php,
  python,
  r,
  ruby,
  rust,
  scala,
  scss,
  sql,
  swift,
  typescript,
  xml,
  yaml,
} as const;

for (const [name, language] of Object.entries(languages)) {
  hljs.registerLanguage(name, language);
}
hljs.registerLanguage("sass", scss);
hljs.registerLanguage("svelte", xml);
hljs.registerLanguage("vue", xml);
hljs.registerLanguage("zig", cpp);

interface HighlightRequest {
  type: "highlight";
  requestId: string;
  language: string;
  hunks: Array<{
    id: string;
    leftLines: string[];
    rightLines: string[];
  }>;
}

interface HighlightResponse {
  type: "result";
  requestId: string;
  hunks: Array<{
    id: string;
    leftHtml: string[];
    rightHtml: string[];
  }>;
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function splitHighlightedLines(html: string, lineCount: number): string[] {
  const lines: string[] = [];
  const openTags: { name: string; source: string }[] = [];
  const tokens = html.match(/<\/?[^>]+>|[^<]+/g) ?? [];
  let current = "";

  const closeTags = () => {
    for (let i = openTags.length - 1; i >= 0; i--) {
      current += `</${openTags[i].name}>`;
    }
  };

  const reopenTags = () => {
    for (const tag of openTags) current += tag.source;
  };

  for (const token of tokens) {
    if (token.startsWith("<")) {
      const closing = token.match(/^<\/([a-zA-Z][\w:-]*)\s*>$/);
      if (closing) {
        current += token;
        const index = openTags.map((tag) => tag.name).lastIndexOf(closing[1]);
        if (index >= 0) openTags.splice(index, 1);
        continue;
      }

      current += token;
      const opening = token.match(/^<([a-zA-Z][\w:-]*)(?:\s[^>]*)?>$/);
      if (opening && !token.endsWith("/>") && !["br", "hr", "img"].includes(opening[1])) {
        openTags.push({ name: opening[1], source: token });
      }
      continue;
    }

    let start = 0;
    while (start <= token.length) {
      const newline = token.indexOf("\n", start);
      if (newline === -1) {
        current += token.slice(start);
        break;
      }
      current += token.slice(start, newline);
      closeTags();
      lines.push(current);
      current = "";
      reopenTags();
      start = newline + 1;
    }
  }

  closeTags();
  lines.push(current);
  return Array.from({ length: lineCount }, (_, index) => lines[index] ?? "");
}

function highlightLines(lines: string[], language: string): string[] {
  try {
    const result = hljs.highlight(lines.join("\n"), {
      language,
      ignoreIllegals: true,
    });
    return splitHighlightedLines(result.value, lines.length);
  } catch {
    return lines.map(escapeHtml);
  }
}

self.onmessage = (event: MessageEvent<HighlightRequest>) => {
  if (event.data.type !== "highlight") return;

  const response: HighlightResponse = {
    type: "result",
    requestId: event.data.requestId,
    hunks: event.data.hunks.map((hunk) => ({
      id: hunk.id,
      leftHtml: highlightLines(hunk.leftLines, event.data.language),
      rightHtml: highlightLines(hunk.rightLines, event.data.language),
    })),
  };
  self.postMessage(response);
};
