import { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { api } from "../api/client";
import { commandFragment, getSuggestions } from "../commands";
import type { ModelInfo, FileSuggestion } from "../types/api";
import type { CommandSuggestion } from "../commands";

export type ThinkingOption = { label: string; value: string };
export type SessionMode = "plan" | "build";

export interface UseSmartInputOptions {
  initialModelId?: string | null;
  initialMode?: SessionMode;
  onModelChange?: (model: ModelInfo) => void;
  onModeChange?: (mode: SessionMode) => void;
  onThinkingChange?: (thinking: string) => void;
}

// File mention state type
export type FileMentionState = {
  visible: boolean;
  query: string;
  atPosition: number;
  cursorPosition: { x: number; y: number };
} | null;

// Command palette state type
export type CommandPaletteState = {
  visible: boolean;
  selectedIndex: number;
  suggestions: CommandSuggestion[];
  position: { x: number; y: number };
};

export interface UseSmartInputReturn {
  // Input state
  inputValue: string;
  setInputValue: (value: string) => void;
  isSubmitting: boolean;
  setIsSubmitting: (value: boolean) => void;

  // Mode state
  mode: SessionMode;
  setMode: (mode: SessionMode) => void;
  toggleMode: () => void;

  // Model state
  models: ModelInfo[];
  selectedModelId: string | null;
  selectedProviderId: string | null;
  selectedModelDisplay: ModelInfo | undefined;
  setSelectedModelId: (id: string) => void;
  handleModelSelect: (model: ModelInfo) => void;
  modelDropdownOpen: boolean;
  setModelDropdownOpen: (open: boolean) => void;
  modelSearchQuery: string;
  setModelSearchQuery: (query: string) => void;
  filteredModels: ModelInfo[];
  groupedModels: Map<string, ModelInfo[]>;

  // Thinking state
  thinkingOptions: ThinkingOption[];
  selectedThinking: string;
  setSelectedThinking: (thinking: string) => void;
  thinkingDropdownOpen: boolean;
  setThinkingDropdownOpen: (open: boolean) => void;
  updateThinkingLevels: (modelId: string) => void;

  // File mention (@) state
  fileMention: FileMentionState;
  setFileMention: (mention: FileMentionState) => void;
  handleFileSelect: (path: string, kind: FileSuggestion["kind"]) => void;

  // Command palette (/) state
  commandPalette: CommandPaletteState;
  setCommandPalette: (palette: CommandPaletteState) => void;
  closeCommandPalette: () => void;
  executeCommand: (name: string) => void;

  // Refs
  inputRef: React.RefObject<HTMLTextAreaElement | HTMLInputElement | null>;
  dropdownRef: React.RefObject<HTMLDivElement | null>;
  thinkingDropdownRef: React.RefObject<HTMLDivElement | null>;

  // Handlers
  handleInputChange: (
    e: React.ChangeEvent<HTMLTextAreaElement | HTMLInputElement>,
  ) => void;
  handleKeyDown: (
    e: React.KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>,
    onSubmit?: () => void,
  ) => boolean;

  // Utils
  getSubmitPayload: () => {
    modelId: string | null;
    providerId: string | null;
    mode: SessionMode;
    thinkingLevel: string | null;
    inputValue: string;
  };
}

export function useSmartInput(
  options: UseSmartInputOptions = {},
): UseSmartInputReturn {
  const {
    initialModelId = null,
    initialMode = "build",
    onModelChange,
    onModeChange,
    onThinkingChange,
  } = options;

  // Input state
  const [inputValue, setInputValue] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement | HTMLInputElement | null>(null);

  // Mode state
  const [mode, setModeState] = useState<SessionMode>(initialMode);
  const setMode = useCallback(
    (newMode: SessionMode) => {
      setModeState(newMode);
      onModeChange?.(newMode);
    },
    [onModeChange],
  );
  const toggleMode = useCallback(() => {
    const newMode = mode === "plan" ? "build" : "plan";
    setMode(newMode);
  }, [mode, setMode]);

  // Model state
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelDropdownOpen, setModelDropdownOpen] = useState(false);
  const [modelSearchQuery, setModelSearchQuery] = useState("");
  const [selectedModelId, setSelectedModelId] = useState<string | null>(
    initialModelId,
  );
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(
    null,
  );

  // Thinking state
  const [thinkingOptions, setThinkingOptions] = useState<ThinkingOption[]>([]);
  const [selectedThinking, setSelectedThinkingState] = useState<string>("");
  const [thinkingDropdownOpen, setThinkingDropdownOpen] = useState(false);
  const setSelectedThinking = useCallback(
    (thinking: string) => {
      setSelectedThinkingState(thinking);
      onThinkingChange?.(thinking);
    },
    [onThinkingChange],
  );

  // File mention (@) state
  type FileMentionState = {
    visible: boolean;
    query: string;
    atPosition: number;
    cursorPosition: { x: number; y: number };
  } | null;
  const [fileMention, setFileMention] = useState<FileMentionState>(null);

  // Command palette (/) state
  type CommandPaletteState = {
    visible: boolean;
    selectedIndex: number;
    suggestions: CommandSuggestion[];
    position: { x: number; y: number };
  };
  const [commandPalette, setCommandPalette] = useState<CommandPaletteState>({
    visible: false,
    selectedIndex: 0,
    suggestions: [],
    position: { x: 0, y: 0 },
  });
  // Refs
  const dropdownRef = useRef<HTMLDivElement | null>(null);
  const thinkingDropdownRef = useRef<HTMLDivElement | null>(null);

  // Update thinking levels based on model - use data from API model info
  const updateThinkingLevels = useCallback(
    (modelId: string) => {
      const model = models.find((m) => m.id === modelId);
      if (
        model &&
        model.thinking_supported &&
        model.thinking_options.length > 0
      ) {
        const options = model.thinking_options.map((opt) => {
          const parts = opt.split(":");
          const label = parts[1]
            ? parts[1].charAt(0).toUpperCase() + parts[1].slice(1)
            : opt;
          return { label, value: opt };
        });
        setThinkingOptions(options);
        const defaultTl = model.thinking_options.includes(model.thinking_level)
          ? model.thinking_level
          : model.thinking_options[0];
        setSelectedThinkingState(defaultTl);
      } else {
        setThinkingOptions([]);
        setSelectedThinkingState("");
      }
    },
    [models],
  );

  // Fetch models and default model on mount
  useEffect(() => {
    Promise.all([api.listModels(), api.getDefaultModel().catch(() => null)])
      .then(([{ models }, defaultModel]) => {
        setModels(models);

        // Select model: use initialModelId, or config default, or first available
        if (!selectedModelId && models.length > 0) {
          let modelToSelect: ModelInfo | undefined;

          if (initialModelId) {
            // Use explicitly provided initial model
            modelToSelect = models.find((m) => m.id === initialModelId);
          }

          if (!modelToSelect && defaultModel) {
            // Use config default model
            modelToSelect = models.find(
              (m) =>
                m.id === defaultModel.model_id &&
                m.provider_id === defaultModel.provider_id,
            );
          }

          if (!modelToSelect) {
            // Fall back to first available model
            modelToSelect = models[0];
          }

          if (modelToSelect) {
            setSelectedModelId(modelToSelect.id);
            setSelectedProviderId(modelToSelect.provider_id);
            updateThinkingLevels(modelToSelect.id);
            onModelChange?.(modelToSelect);
          }
        }
      })
      .catch(console.error);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Filtered models
  const filteredModels = useMemo(() => {
    if (!modelSearchQuery.trim()) return models;
    const q = modelSearchQuery.toLowerCase();
    return models.filter(
      (m) =>
        m.display_name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.provider_name.toLowerCase().includes(q),
    );
  }, [models, modelSearchQuery]);

  // Grouped models
  const groupedModels = useMemo(() => {
    const groups = new Map<string, ModelInfo[]>();
    for (const m of filteredModels) {
      const key = m.provider_name || m.provider_id;
      if (!groups.has(key)) {
        groups.set(key, []);
      }
      groups.get(key)!.push(m);
    }
    return groups;
  }, [filteredModels]);

  // Selected model display
  const selectedModelDisplay = useMemo(
    () => models.find((m) => m.id === selectedModelId),
    [models, selectedModelId],
  );

  // Handle model selection
  const handleModelSelect = useCallback(
    (model: ModelInfo) => {
      setSelectedModelId(model.id);
      setSelectedProviderId(model.provider_id);
      setModelDropdownOpen(false);
      setModelSearchQuery("");
      updateThinkingLevels(model.id);

      // Save as default model in config
      api
        .setDefaultModel({
          provider_id: model.provider_id,
          model_id: model.id,
        })
        .catch(() => {
          // Silently fail - non-critical operation
        });

      onModelChange?.(model);
    },
    [onModelChange, updateThinkingLevels],
  );

  // Close command palette
  const closeCommandPalette = useCallback(() => {
    setCommandPalette({
      visible: false,
      selectedIndex: 0,
      suggestions: [],
      position: { x: 0, y: 0 },
    });
  }, []);

  // Execute command
  const executeCommand = useCallback(
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    (_name: string) => {
      setInputValue("");
      closeCommandPalette();
      // Commands are handled by the consumer via onSlashCommand prop
    },
    [closeCommandPalette],
  );

  // Handle file selection from @ mention
  const handleFileSelect = useCallback(
    (path: string, kind: FileSuggestion["kind"]) => {
      if (!fileMention) return;

      const before = inputValue.slice(0, fileMention.atPosition);
      const after = inputValue.slice(
        fileMention.atPosition + 1 + fileMention.query.length,
      );
      const replacement = kind === "directory" ? `@${path}/` : `@${path}`;
      const newValue = `${before}${replacement}${after ? " " + after : ""}`;
      setInputValue(newValue);
      setFileMention(null);

      // Focus input after selection
      setTimeout(() => inputRef.current?.focus(), 0);
    },
    [inputValue, fileMention],
  );

  // Handle input change with @ and / detection
  const handleInputChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement | HTMLInputElement>) => {
      const newValue = e.target.value;
      const input = e.target;
      setInputValue(newValue);

      // Get cursor position
      const cursorPosition =
        "selectionStart" in input ? (input.selectionStart ?? 0) : 0;

      // Check for @ mention
      const textBeforeCursor = newValue.slice(0, cursorPosition);
      const atMatch = textBeforeCursor.match(/@([^\s]*)$/);

      if (atMatch) {
        const query = atMatch[1] || "";
        const atPosition = textBeforeCursor.lastIndexOf("@");

        // Get cursor coordinates
        if ("getBoundingClientRect" in input) {
          const rect = input.getBoundingClientRect();
          setFileMention({
            visible: true,
            query,
            atPosition,
            cursorPosition: { x: rect.left + 20, y: rect.top },
          });
        }
      } else {
        setFileMention(null);
      }

      // Check for / command
      const fragment = commandFragment(newValue);
      if (fragment !== null) {
        const suggestions = getSuggestions(fragment);
        if (suggestions.length > 0) {
          if ("getBoundingClientRect" in input) {
            const rect = input.getBoundingClientRect();
            setCommandPalette({
              visible: true,
              selectedIndex: 0,
              suggestions,
              position: { x: rect.left, y: rect.top },
            });
          }
        } else {
          closeCommandPalette();
        }
      } else {
        closeCommandPalette();
      }
    },
    [closeCommandPalette],
  );

  // Handle keydown for navigation and submit
  const handleKeyDown = useCallback(
    (
      e: React.KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>,
      onSubmit?: () => void,
    ): boolean => {
      // Handle command palette navigation
      if (commandPalette.visible) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setCommandPalette((prev) => ({
            ...prev,
            selectedIndex: (prev.selectedIndex + 1) % prev.suggestions.length,
          }));
          return true;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setCommandPalette((prev) => ({
            ...prev,
            selectedIndex:
              prev.selectedIndex === 0
                ? prev.suggestions.length - 1
                : prev.selectedIndex - 1,
          }));
          return true;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          const suggestion =
            commandPalette.suggestions[commandPalette.selectedIndex];
          if (suggestion) {
            executeCommand(suggestion.spec.name);
          }
          return true;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          closeCommandPalette();
          return true;
        }
        return false;
      }

      // Handle file mention navigation
      if (fileMention?.visible) {
        // Let the FileMentionPopover handle its own keyboard navigation
        if (e.key === "Escape") {
          e.preventDefault();
          setFileMention(null);
          return true;
        }
        return false;
      }

      // Handle submit on Enter (without shift for textarea)
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        if (onSubmit && inputValue.trim() && !isSubmitting) {
          onSubmit();
        }
        return true;
      }

      return false;
    },
    [
      commandPalette,
      fileMention,
      inputValue,
      isSubmitting,
      executeCommand,
      closeCommandPalette,
    ],
  );

  // Get submit payload
  const getSubmitPayload = useCallback(() => {
    return {
      modelId: selectedModelId,
      providerId: selectedProviderId,
      mode,
      thinkingLevel: selectedThinking || null,
      inputValue: inputValue.trim(),
    };
  }, [selectedModelId, selectedProviderId, mode, selectedThinking, inputValue]);

  return {
    // Input state
    inputValue,
    setInputValue,
    isSubmitting,
    setIsSubmitting,

    // Mode state
    mode,
    setMode,
    toggleMode,

    // Model state
    models,
    selectedModelId,
    selectedProviderId,
    selectedModelDisplay,
    setSelectedModelId,
    handleModelSelect,
    modelDropdownOpen,
    setModelDropdownOpen,
    modelSearchQuery,
    setModelSearchQuery,
    filteredModels,
    groupedModels,

    // Thinking state
    thinkingOptions,
    selectedThinking,
    setSelectedThinking,
    thinkingDropdownOpen,
    setThinkingDropdownOpen,
    updateThinkingLevels,

    // File mention state
    fileMention,
    setFileMention,
    handleFileSelect,

    // Command palette state
    commandPalette,
    setCommandPalette,
    closeCommandPalette,
    executeCommand,

    // Refs
    inputRef,
    dropdownRef,
    thinkingDropdownRef,

    // Handlers
    handleInputChange,
    handleKeyDown,

    // Utils
    getSubmitPayload,
  };
}
