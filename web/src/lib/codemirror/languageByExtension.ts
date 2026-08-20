import type { LanguageSupport } from "@codemirror/language";

type AsyncLangLoader = () => Promise<LanguageSupport>;

/**
 * Map of file extensions to async language loaders.
 * Uses dynamic imports to avoid circular dependencies in the bundle.
 */
const asyncLanguages: Record<string, AsyncLangLoader> = {
  js: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  jsx: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ jsx: true })),
  mjs: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  ts: () => import("@codemirror/lang-javascript").then((m) => m.javascript({ typescript: true })),
  tsx: () =>
    import("@codemirror/lang-javascript").then((m) =>
      m.javascript({ jsx: true, typescript: true }),
    ),
  py: () => import("@codemirror/lang-python").then((m) => m.python()),
  rs: () => import("@codemirror/lang-rust").then((m) => m.rust()),
  json: () => import("@codemirror/lang-json").then((m) => m.json()),
  md: () => import("@codemirror/lang-markdown").then((m) => m.markdown()),
  markdown: () => import("@codemirror/lang-markdown").then((m) => m.markdown()),
  css: () => import("@codemirror/lang-css").then((m) => m.css()),
  scss: () => import("@codemirror/lang-css").then((m) => m.css()),
  less: () => import("@codemirror/lang-css").then((m) => m.css()),
  html: () => import("@codemirror/lang-html").then((m) => m.html()),
  htm: () => import("@codemirror/lang-html").then((m) => m.html()),
  sql: () => import("@codemirror/lang-sql").then((m) => m.sql()),
  xml: () => import("@codemirror/lang-xml").then((m) => m.xml()),
  svg: () => import("@codemirror/lang-xml").then((m) => m.xml()),
  yaml: () => import("@codemirror/lang-yaml").then((m) => m.yaml()),
  yml: () => import("@codemirror/lang-yaml").then((m) => m.yaml()),
  c: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  h: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  cpp: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  hpp: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  cc: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  cxx: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
  hxx: () => import("@codemirror/lang-cpp").then((m) => m.cpp()),
};

/**
 * Asynchronously load a CodeMirror language by file extension.
 * Tries pre-loaded common languages first, then falls back to @codemirror/language-data.
 */
export async function loadLanguageByExtension(filePath: string): Promise<LanguageSupport | null> {
  const ext = filePath.split(".").pop()?.toLowerCase() || "";
  if (!ext) return null;

  // Check common languages map first
  const asyncLoader = asyncLanguages[ext];
  if (asyncLoader) return await asyncLoader();

  // Dynamic loading via language-data
  try {
    const { languages } = await import("@codemirror/language-data");

    // Try to find a matching language by extension
    for (const lang of languages) {
      if (lang.extensions?.includes(ext) || lang.extensions?.includes("." + ext)) {
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
