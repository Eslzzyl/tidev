import { useEffect, useRef, useState, forwardRef, useImperativeHandle } from "react";
import { EditorState, Compartment, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter } from "@codemirror/view";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { closeBrackets } from "@codemirror/autocomplete";
import { bracketMatching, foldGutter } from "@codemirror/language";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { createCodeMirrorTheme } from "../../lib/codemirror/theme";
import { loadLanguageByExtension } from "../../lib/codemirror/languageByExtension";

export interface CodeMirrorEditorProps {
  /** File content */
  value: string;
  /** Called when content changes */
  onChange?: (value: string) => void;
  /** File path for language detection */
  filePath?: string;
  /** Whether the editor is read-only */
  readOnly?: boolean;
  /** Additional CodeMirror extensions */
  extensions?: Extension[];
  /** Class name for the container */
  className?: string;
  /** Whether the editor is dark mode */
  dark?: boolean;
  /** Called when the editor view is created */
  onViewReady?: (view: EditorView) => void;
  /** Called when the editor view is destroyed */
  onViewDestroy?: () => void;
}

export interface CodeMirrorEditorHandle {
  /** Scroll to a specific line number */
  goToLine: (line: number) => void;
  /** Get the total line count */
  getLineCount: () => number;
  /** Get the current line number */
  getCurrentLine: () => number;
}

/**
 * A CodeMirror 6 editor component for React 19.
 * Exposes goToLine, getLineCount, getCurrentLine via ref.
 */
export const CodeMirrorEditor = forwardRef<CodeMirrorEditorHandle, CodeMirrorEditorProps>(function CodeMirrorEditor({
  value,
  onChange,
  filePath,
  readOnly = true,
  extensions: externalExtensions = [],
  className = "",
  dark = false,
  onViewReady,
  onViewDestroy,
}, ref) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [isReady, setIsReady] = useState(false);

  // Compartments for dynamic configuration
  const themeCompartment = useRef(new Compartment());
  const languageCompartment = useRef(new Compartment());
  const editableCompartment = useRef(new Compartment());
  const externalExtensionsCompartment = useRef(new Compartment());

  // Track the current value to avoid unnecessary updates
  const currentValueRef = useRef(value);

  // Expose imperative methods
  useImperativeHandle(ref, () => ({
    goToLine(line: number) {
      const view = viewRef.current;
      if (!view) return;
      const doc = view.state.doc;
      const clampedLine = Math.max(1, Math.min(line, doc.lines));
      const pos = doc.line(clampedLine).from;
      view.dispatch({
        selection: { anchor: pos },
        scrollIntoView: true,
      });
      view.focus();
    },
    getLineCount() {
      return viewRef.current?.state.doc.lines ?? 0;
    },
    getCurrentLine() {
      if (!viewRef.current) return 0;
      const pos = viewRef.current.state.selection.main.head;
      return viewRef.current.state.doc.lineAt(pos).number;
    },
  }));

  // Create the editor
  useEffect(() => {
    if (!containerRef.current) return;

    // Create initial state
    const state = EditorState.create({
      doc: value,
      extensions: [
        // Line numbers
        lineNumbers(),
        // Highlight active line
        highlightActiveLine(),
        highlightActiveLineGutter(),
        // Bracket matching
        bracketMatching(),
        // Fold gutter
        foldGutter(),
        // Close brackets
        closeBrackets(),
        // Highlight selection matches
        highlightSelectionMatches(),
        // Keymaps
        keymap.of([
          ...defaultKeymap,
          ...searchKeymap,
          indentWithTab,
        ]),
        // Theme (dynamic)
        themeCompartment.current.of(createCodeMirrorTheme(dark)),
        // Language (dynamic)
        languageCompartment.current.of([]),
        // Editable (dynamic)
        editableCompartment.current.of(EditorView.editable.of(!readOnly)),
        // External extensions (dynamic)
        externalExtensionsCompartment.current.of(externalExtensions),
        // Update listener
        EditorView.updateListener.of((update) => {
          if (update.docChanged && onChange) {
            const doc = update.state.doc.toString();
            currentValueRef.current = doc;
            onChange(doc);
          }
        }),
        // Dom event handling for Ctrl+S and Ctrl+G
        EditorView.domEventHandlers({
          keydown: (event) => {
            const isMod = event.ctrlKey || event.metaKey;
            if (isMod && event.key === "s") {
              event.preventDefault();
              containerRef.current?.dispatchEvent(
                new CustomEvent("editor-save", {
                  detail: { content: currentValueRef.current },
                }),
              );
              return true;
            }
            if (isMod && event.key === "g") {
              event.preventDefault();
              containerRef.current?.dispatchEvent(
                new CustomEvent("editor-gotoline"),
              );
              return true;
            }
            return false;
          },
        }),
      ],
    });

    // Create the view
    const view = new EditorView({
      state,
      parent: containerRef.current,
    });

    viewRef.current = view;
    setIsReady(true);
    onViewReady?.(view);

    return () => {
      onViewDestroy?.();
      view.destroy();
      viewRef.current = null;
      setIsReady(false);
    };
    // Only run on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Update theme
  useEffect(() => {
    if (viewRef.current) {
      viewRef.current.dispatch({
        effects: themeCompartment.current.reconfigure(
          createCodeMirrorTheme(dark),
        ),
      });
    }
  }, [dark]);

  // Update editable state
  useEffect(() => {
    if (viewRef.current) {
      viewRef.current.dispatch({
        effects: editableCompartment.current.reconfigure(
          EditorView.editable.of(!readOnly),
        ),
      });
    }
  }, [readOnly]);

  // Update external extensions
  useEffect(() => {
    if (viewRef.current) {
      viewRef.current.dispatch({
        effects: externalExtensionsCompartment.current.reconfigure(
          externalExtensions,
        ),
      });
    }
  }, [externalExtensions]);

  // Update language when filePath changes
  useEffect(() => {
    if (!viewRef.current) return;

    let cancelled = false;

    async function updateLanguage() {
      let lang = null;

      if (filePath) {
        lang = await loadLanguageByExtension(filePath);
      }

      if (!cancelled && viewRef.current) {
        viewRef.current.dispatch({
          effects: languageCompartment.current.reconfigure(
            lang ? [lang] : [],
          ),
        });
      }
    }

    updateLanguage();

    return () => {
      cancelled = true;
    };
  }, [filePath]);

  // Update document content when value changes externally (but not when it's our own change)
  useEffect(() => {
    if (viewRef.current && isReady) {
      const currentDoc = viewRef.current.state.doc.toString();
      if (currentDoc !== value) {
        viewRef.current.dispatch({
          changes: {
            from: 0,
            to: currentDoc.length,
            insert: value,
          },
        });
        currentValueRef.current = value;
      }
    }
  }, [value, isReady]);

  return (
    <div
      ref={containerRef}
      className={`codemirror-editor h-full overflow-auto ${className}`}
    />
  );
});
