import { useReducer, useEffect, useRef, useMemo, useCallback } from "react";
import { X, Search, BookOpen, FileText, Loader2, AlertCircle } from "lucide-react";
import { api } from "../../api/client";
import type { SkillInfo } from "../../types/api";

interface SkillsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSelect: (skillName: string) => void;
}

/* ─── Reducer for all dialog state ─── */

interface DialogState {
  skills: SkillInfo[];
  loading: boolean;
  error: string | null;
  searchQuery: string;
  selectedIndex: number;
}

type Action =
  | { type: "FETCH_START" }
  | { type: "FETCH_SUCCESS"; skills: SkillInfo[] }
  | { type: "FETCH_ERROR"; error: string }
  | { type: "SET_SEARCH"; query: string }
  | { type: "SELECT_INDEX"; index: number }
  | { type: "NAV_UP"; max: number }
  | { type: "NAV_DOWN"; max: number };

function reducer(state: DialogState, action: Action): DialogState {
  switch (action.type) {
    case "FETCH_START":
      return { ...state, loading: true, error: null };
    case "FETCH_SUCCESS":
      return { ...state, skills: action.skills, loading: false };
    case "FETCH_ERROR":
      return { ...state, error: action.error, loading: false };
    case "SET_SEARCH":
      return { ...state, searchQuery: action.query, selectedIndex: 0 };
    case "SELECT_INDEX":
      return { ...state, selectedIndex: action.index };
    case "NAV_UP": {
      const prev = state.selectedIndex;
      return { ...state, selectedIndex: prev > 0 ? prev - 1 : action.max - 1 };
    }
    case "NAV_DOWN": {
      const prev = state.selectedIndex;
      return { ...state, selectedIndex: prev < action.max - 1 ? prev + 1 : 0 };
    }
  }
}

const initialState: DialogState = {
  skills: [],
  loading: true,
  error: null,
  searchQuery: "",
  selectedIndex: 0,
};

/* ─── Component ─── */

export function SkillsDialog({ isOpen, onClose, onSelect }: SkillsDialogProps) {
  const [state, dispatch] = useReducer(reducer, initialState);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Fetch skills and focus input when dialog opens
  useEffect(() => {
    if (!isOpen) return;

    dispatch({ type: "FETCH_START" });

    api
      .listSkills()
      .then((data) => dispatch({ type: "FETCH_SUCCESS", skills: data.skills }))
      .catch((err) =>
        dispatch({
          type: "FETCH_ERROR",
          error: err instanceof Error ? err.message : "Failed to load skills",
        }),
      );

    const raf = requestAnimationFrame(() => {
      searchInputRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [isOpen]);

  // Filter skills by search query
  const filteredSkills = useMemo(() => {
    if (!state.searchQuery.trim()) return state.skills;
    const query = state.searchQuery.trim().toLowerCase();
    return state.skills.filter(
      (s) => s.name.toLowerCase().includes(query) || s.description.toLowerCase().includes(query),
    );
  }, [state.skills, state.searchQuery]);

  // Selected skill (for preview)
  const selectedSkill = useMemo(() => {
    if (filteredSkills.length === 0) return null;
    const idx = Math.min(state.selectedIndex, filteredSkills.length - 1);
    return filteredSkills[idx] ?? null;
  }, [filteredSkills, state.selectedIndex]);

  function handleSkillClick(skill: SkillInfo) {
    onSelect(skill.name);
  }

  // Handle arrow keys, Enter, and ESC
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const max = filteredSkills.length;
      if (max === 0) return;

      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          dispatch({ type: "NAV_DOWN", max });
          break;
        case "ArrowUp":
          e.preventDefault();
          dispatch({ type: "NAV_UP", max });
          break;
        case "Enter":
          e.preventDefault();
          if (selectedSkill) onSelect(selectedSkill.name);
          break;
      }
    },
    [filteredSkills.length, selectedSkill, onSelect],
  );

  // Close on ESC key
  useEffect(() => {
    if (!isOpen) return;

    function handleEsc(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }

    document.addEventListener("keydown", handleEsc);
    return () => document.removeEventListener("keydown", handleEsc);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 motion-safe:animate-fade-in p-4">
      <div
        className="motion-safe:animate-scale-fade flex h-[85vh] w-full max-w-3xl flex-col rounded-lg bg-white shadow-lg dark:bg-neutral-900"
        onKeyDown={handleKeyDown}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
          <div className="flex items-center gap-2">
            <BookOpen className="h-5 w-5 text-neutral-500 dark:text-neutral-400" />
            <h3 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">Skills</h3>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1 text-neutral-400 transition-all duration-150 hover:bg-neutral-100 hover:text-neutral-600 active:scale-95 dark:hover:bg-neutral-800 dark:hover:text-neutral-300"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Search input */}
        <div className="border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-neutral-400" />
            <input
              ref={searchInputRef}
              type="text"
              value={state.searchQuery}
              onChange={(e) => dispatch({ type: "SET_SEARCH", query: e.target.value })}
              placeholder="Search skills..."
              className="w-full rounded-lg border border-neutral-300 bg-neutral-50 py-2 pl-10 pr-4 text-base text-neutral-900 placeholder-neutral-400 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 dark:placeholder-neutral-500 dark:focus:border-blue-400"
            />
          </div>
        </div>

        {/* Content area */}
        {state.loading ? (
          <div className="flex flex-1 items-center justify-center">
            <div className="flex flex-col items-center gap-2 text-neutral-500 dark:text-neutral-400">
              <Loader2 className="h-6 w-6 animate-spin" />
              <span className="text-sm">Loading skills...</span>
            </div>
          </div>
        ) : state.error ? (
          <div className="flex flex-1 items-center justify-center">
            <div className="flex flex-col items-center gap-2 text-red-500">
              <AlertCircle className="h-6 w-6" />
              <span className="text-sm">{state.error}</span>
            </div>
          </div>
        ) : state.skills.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-neutral-500 dark:text-neutral-400">
            <BookOpen className="h-10 w-10" />
            <p className="text-sm font-medium">No skills discovered</p>
            <p className="max-w-xs text-center text-xs">
              Create SKILL.md files in your project's
              <br />
              <code className="rounded bg-neutral-100 px-1 py-0.5 text-xs dark:bg-neutral-800">
                .opencode/skills/
              </code>
              ,{" "}
              <code className="rounded bg-neutral-100 px-1 py-0.5 text-xs dark:bg-neutral-800">
                .claude/skills/
              </code>
              , or{" "}
              <code className="rounded bg-neutral-100 px-1 py-0.5 text-xs dark:bg-neutral-800">
                .agents/skills/
              </code>{" "}
              directories.
            </p>
          </div>
        ) : filteredSkills.length === 0 ? (
          <div className="flex flex-1 items-center justify-center">
            <p className="text-sm text-neutral-500 dark:text-neutral-400">
              No skills match "{state.searchQuery}"
            </p>
          </div>
        ) : (
          <div className="flex flex-1 flex-col overflow-hidden md:flex-row">
            {/* Skill list (left pane on wide, full on narrow) */}
            <div className="flex flex-1 flex-col overflow-hidden md:w-1/2 md:border-r md:border-neutral-200 md:dark:border-neutral-800">
              <div className="flex-1 overflow-y-auto">
                {filteredSkills.map((skill, index) => (
                  <button
                    key={skill.name}
                    onClick={() => handleSkillClick(skill)}
                    onMouseEnter={() => dispatch({ type: "SELECT_INDEX", index })}
                    className={`w-full px-4 py-3 text-left transition-all duration-150 active:scale-[0.99] ${
                      index === state.selectedIndex
                        ? "bg-blue-50 dark:bg-blue-900/30"
                        : "hover:bg-neutral-50 dark:hover:bg-neutral-800/50"
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span
                        className={`text-sm font-semibold ${
                          index === state.selectedIndex
                            ? "text-blue-700 dark:text-blue-300"
                            : "text-neutral-900 dark:text-neutral-100"
                        }`}
                      >
                        {skill.name}
                      </span>
                      {index === state.selectedIndex && (
                        <span className="hidden text-xs text-blue-500 md:inline dark:text-blue-400">
                          Enter to load
                        </span>
                      )}
                    </div>
                    <p className="mt-0.5 text-xs text-neutral-500 line-clamp-2 dark:text-neutral-400">
                      {skill.description || "No description"}
                    </p>
                  </button>
                ))}
              </div>
              {/* Footer count */}
              <div className="border-t border-neutral-200 px-4 py-2 text-xs text-neutral-400 dark:border-neutral-800">
                {filteredSkills.length} of {state.skills.length} skills
              </div>
            </div>

            {/* Preview pane (right pane on wide, bottom on narrow) */}
            {selectedSkill && (
              <div className="flex flex-col border-t border-neutral-200 md:w-1/2 md:border-t-0 md:border-l dark:border-neutral-800">
                <div className="flex items-center gap-2 border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
                  <FileText className="h-4 w-4 text-neutral-500 dark:text-neutral-400" />
                  <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    Preview
                  </span>
                  <span className="ml-auto text-xs text-neutral-400">{selectedSkill.name}</span>
                </div>
                <div className="flex-1 overflow-y-auto p-4">
                  <div className="mb-3">
                    <h4 className="text-sm font-bold text-neutral-900 dark:text-neutral-100">
                      {selectedSkill.name}
                    </h4>
                    <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
                      {selectedSkill.description || "No description"}
                    </p>
                  </div>
                  <div className="rounded-md bg-neutral-50 p-3 dark:bg-neutral-800">
                    <p className="text-xs text-neutral-500 dark:text-neutral-400">Location:</p>
                    <code className="mt-1 block break-all text-xs text-neutral-700 dark:text-neutral-300">
                      {selectedSkill.location}
                    </code>
                  </div>
                  <div className="mt-4">
                    <button
                      onClick={() => onSelect(selectedSkill.name)}
                      className="w-full rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-all duration-150 hover:bg-blue-700 active:scale-[0.98] dark:bg-blue-700 dark:hover:bg-blue-600"
                    >
                      Load "{selectedSkill.name}"
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        {/* Keyboard hints footer */}
        {!state.loading && !state.error && filteredSkills.length > 0 && (
          <div className="hidden border-t border-neutral-200 px-4 py-2 text-xs text-neutral-400 md:block dark:border-neutral-800">
            <kbd className="rounded border border-neutral-300 px-1 font-mono text-[10px] dark:border-neutral-600">
              ↑↓
            </kbd>{" "}
            Navigate ·{" "}
            <kbd className="rounded border border-neutral-300 px-1 font-mono text-[10px] dark:border-neutral-600">
              Enter
            </kbd>{" "}
            Load skill ·{" "}
            <kbd className="rounded border border-neutral-300 px-1 font-mono text-[10px] dark:border-neutral-600">
              Esc
            </kbd>{" "}
            Close
          </div>
        )}
      </div>
    </div>
  );
}
