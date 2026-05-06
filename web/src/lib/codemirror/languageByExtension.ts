import type { LanguageSupport } from "@codemirror/language";

// Synchronous imports for commonly used languages
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { json as jsonLang } from "@codemirror/lang-json";
import { markdown as mdLang } from "@codemirror/lang-markdown";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml as yamlLang } from "@codemirror/lang-yaml";
import { cpp } from "@codemirror/lang-cpp";

type LangLoader = () => LanguageSupport;

/**
 * Map of file extensions to synchronous (pre-loaded) language loaders.
 * The loader function returns a LanguageSupport for CodeMirror 6.
 */
const syncLanguages: Record<string, LangLoader> = {
  js: () => javascript(),
  jsx: () => javascript({ jsx: true }),
  mjs: () => javascript(),
  ts: () => javascript({ typescript: true }),
  tsx: () => javascript({ jsx: true, typescript: true }),
  py: () => python(),
  rs: () => rust(),
  json: () => jsonLang(),
  md: () => mdLang(),
  markdown: () => mdLang(),
  css: () => css(),
  scss: () => css(),
  less: () => css(),
  html: () => html(),
  htm: () => html(),
  sql: () => sql(),
  xml: () => xml(),
  svg: () => xml(),
  yaml: () => yamlLang(),
  yml: () => yamlLang(),
  c: () => cpp(),
  h: () => cpp(),
  cpp: () => cpp(),
  hpp: () => cpp(),
  cc: () => cpp(),
  cxx: () => cpp(),
  hxx: () => cpp(),
};

/**
 * Get the CodeMirror language for a given file path synchronously.
 * Returns the LanguageSupport or null if not found in pre-loaded set.
 */
export function languageByExtension(filePath: string): LanguageSupport | null {
  const ext = filePath.split(".").pop()?.toLowerCase() || "";
  if (!ext) return null;
  const loader = syncLanguages[ext];
  if (loader) return loader();
  return null;
}

/**
 * Asynchronously load a CodeMirror language by file extension using @codemirror/language-data.
 */
export async function loadLanguageByExtension(
  filePath: string,
): Promise<LanguageSupport | null> {
  const ext = filePath.split(".").pop()?.toLowerCase() || "";
  if (!ext) return null;

  // Check sync map first
  const syncLoader = syncLanguages[ext];
  if (syncLoader) return syncLoader();

  // Dynamic loading via language-data
  try {
    const { languages } = await import("@codemirror/language-data");

    // Try to find a matching language by extension
    for (const lang of languages) {
      if (
        lang.extensions?.includes(ext) ||
        lang.extensions?.includes("." + ext)
      ) {
        return lang.support ?? (await lang.load()) ?? null;
      }
    }

    // Try by filename
    const fileName = filePath.split("/").pop() || filePath;
    for (const lang of languages) {
      if (lang.filename?.test?.(fileName)) {
        return lang.support ?? (await lang.load()) ?? null;
      }
    }
  } catch {
    // Fall through to null
  }

  return null;
}
