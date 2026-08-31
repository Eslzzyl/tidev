import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Sparkles,
  Search,
  BookOpen,
  FileCode,
  Files,
  Copy,
  Check,
  Send,
  RefreshCw,
  X,
  Package,
  FolderGit2,
} from "lucide-react";
import { useSkillsQuery, useSkillFileQuery } from "../../hooks/workspaceQueries";
import { useUIStore } from "../../stores/useUIStore";
import { MarkdownRenderer } from "../renderers/MarkdownRenderer";
import { Button, IconButton, Input, Tabs } from "../ui";
import type { SkillInfo } from "../../types/api";

type SkillFilter = "all" | "bundled" | "custom";

export function SkillsSection() {
  const { t } = useTranslation();
  const { data, isLoading, error, refetch } = useSkillsQuery();
  const closeSettingsPanel = useUIStore((s) => s.closeSettingsPanel);
  const setPendingDraft = useUIStore((s) => s.setPendingDraft);

  const [searchQuery, setSearchQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<SkillFilter>("all");
  const [selectedSkillName, setSelectedSkillName] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<"doc" | "companions">("doc");
  const [selectedCompanionPath, setSelectedCompanionPath] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const skills: SkillInfo[] = useMemo(() => data?.skills || [], [data?.skills]);

  const filteredSkills = useMemo(() => {
    return skills.filter((skill) => {
      // Filter by category
      if (activeFilter === "bundled" && !skill.is_bundled) return false;
      if (activeFilter === "custom" && skill.is_bundled) return false;

      // Filter by query
      if (!searchQuery.trim()) return true;
      const q = searchQuery.toLowerCase();
      return (
        skill.name.toLowerCase().includes(q) ||
        skill.description.toLowerCase().includes(q) ||
        skill.location.toLowerCase().includes(q)
      );
    });
  }, [skills, activeFilter, searchQuery]);

  // Default selected skill
  const selectedSkill = useMemo(() => {
    if (selectedSkillName) {
      const found = skills.find((s) => s.name === selectedSkillName);
      if (found) return found;
    }
    return filteredSkills[0] || null;
  }, [skills, filteredSkills, selectedSkillName]);

  // Query companion file content if active
  const {
    data: companionFileData,
    isLoading: companionFileLoading,
    error: companionFileError,
  } = useSkillFileQuery(
    selectedSkill?.name ?? null,
    activeTab === "companions" && selectedCompanionPath ? selectedCompanionPath : undefined,
  );

  const handleCopy = async (key: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => setCopiedKey(null), 2000);
    } catch (err) {
      console.error("Failed to copy:", err);
    }
  };

  const handleUseInChat = (skill: SkillInfo) => {
    setPendingDraft(`/skill ${skill.name} `);
    closeSettingsPanel();
  };

  const bundledCount = useMemo(() => skills.filter((s) => s.is_bundled).length, [skills]);
  const customCount = useMemo(() => skills.filter((s) => !s.is_bundled).length, [skills]);

  return (
    <section className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
              {t("Skills")}
            </h2>
            <span className="rounded-full bg-neutral-100 px-2 py-0.5 text-[11px] font-medium text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400">
              {skills.length}
            </span>
          </div>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-0.5">
            {t("Browse, preview, and load agent skills and companion documents")}
          </p>
        </div>

        <IconButton
          label={t("Refresh skills")}
          size="sm"
          onClick={() => void refetch()}
          title={t("Refresh skills")}
        >
          <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
        </IconButton>
      </div>

      {/* Search & Filter Bar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-2">
        <div className="relative flex-1">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-neutral-400" />
          <Input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("Search skills by name or description...")}
            size="sm"
            className="skills-search-input"
          />
          {searchQuery && (
            <IconButton
              label={t("Clear search")}
              size="sm"
              onClick={() => setSearchQuery("")}
              className="skills-search-clear"
            >
              <X className="h-3 w-3" />
            </IconButton>
          )}
        </div>

        {/* Filter Pills */}
        <Tabs.Root
          value={activeFilter}
          onValueChange={(value) => setActiveFilter(value as SkillFilter)}
        >
          <Tabs.List className="skills-filter-tabs" aria-label={t("Skill filter")}>
            <Tabs.Trigger value="all">
              {t("All")} ({skills.length})
            </Tabs.Trigger>
            <Tabs.Trigger value="bundled">
              {t("Bundled")} ({bundledCount})
            </Tabs.Trigger>
            <Tabs.Trigger value="custom">
              {t("Custom")} ({customCount})
            </Tabs.Trigger>
          </Tabs.List>
        </Tabs.Root>
      </div>

      {/* Main Content: Split Master-Detail */}
      {isLoading ? (
        <div className="flex items-center justify-center py-16 text-neutral-500">
          <RefreshCw className="h-5 w-5 animate-spin mr-2" />
          <span className="text-sm">{t("Loading skills...")}</span>
        </div>
      ) : error ? (
        <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-400">
          {t("Failed to load skills")}
        </div>
      ) : filteredSkills.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-neutral-300 py-16 text-center dark:border-neutral-800">
          <Sparkles className="h-10 w-10 text-neutral-400 mb-2" />
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
            {t("No skills found")}
          </p>
          <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1 max-w-sm">
            {searchQuery
              ? t("No skills match your search query.")
              : t("No skills discovered in workspace or user directory.")}
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-12 gap-3 min-h-[380px]">
          {/* Left Column: Skill List (5 cols) */}
          <div className="md:col-span-5 flex flex-col space-y-1.5 max-h-[460px] overflow-y-auto pr-1">
            {filteredSkills.map((skill) => {
              const isSelected = selectedSkill?.name === skill.name;

              return (
                <Button
                  type="button"
                  key={skill.name}
                  onClick={() => {
                    setSelectedSkillName(skill.name);
                    setSelectedCompanionPath(skill.companion_files[0] || null);
                  }}
                  className="skill-list-item"
                  variant="secondary"
                  size="md"
                  data-selected={isSelected ? "true" : undefined}
                >
                  <div className="flex items-center justify-between gap-2 w-full">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <Sparkles className="h-3.5 w-3.5 shrink-0 text-amber-500" />
                      <span className="font-mono text-xs font-semibold text-neutral-900 dark:text-neutral-100 truncate">
                        {skill.name}
                      </span>
                    </div>

                    <span
                      className={`inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium shrink-0 ${
                        skill.is_bundled
                          ? "bg-indigo-50 text-indigo-700 dark:bg-indigo-950/50 dark:text-indigo-300 border border-indigo-200/60 dark:border-indigo-800/60"
                          : "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300 border border-emerald-200/60 dark:border-emerald-800/60"
                      }`}
                    >
                      {skill.is_bundled ? (
                        <Package className="h-2.5 w-2.5" />
                      ) : (
                        <FolderGit2 className="h-2.5 w-2.5" />
                      )}
                      {skill.is_bundled ? t("Bundled") : t("Workspace")}
                    </span>
                  </div>

                  {skill.description && (
                    <p className="text-[11px] text-neutral-600 dark:text-neutral-400 line-clamp-2 mt-1.5 leading-relaxed">
                      {skill.description}
                    </p>
                  )}

                  {skill.companion_files && skill.companion_files.length > 0 && (
                    <div className="flex items-center gap-1 text-[10px] text-neutral-400 dark:text-neutral-500 mt-2">
                      <Files className="h-3 w-3" />
                      <span>
                        {t("{{count}} companion files", { count: skill.companion_files.length })}
                      </span>
                    </div>
                  )}
                </Button>
              );
            })}
          </div>

          {/* Right Column: Skill Detail & Markdown Preview (7 cols) */}
          <div className="md:col-span-7 flex flex-col rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900 overflow-hidden">
            {selectedSkill ? (
              <>
                {/* Detail Top Banner */}
                <div className="flex flex-col gap-2 p-3.5 border-b border-neutral-200 bg-neutral-50/50 dark:border-neutral-800 dark:bg-neutral-800/40">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="font-mono text-sm font-bold text-neutral-900 dark:text-neutral-100 truncate">
                        {selectedSkill.name}
                      </span>
                      <span
                        className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] font-medium ${
                          selectedSkill.is_bundled
                            ? "bg-indigo-100/70 text-indigo-700 dark:bg-indigo-900/60 dark:text-indigo-300"
                            : "bg-emerald-100/70 text-emerald-700 dark:bg-emerald-900/60 dark:text-emerald-300"
                        }`}
                      >
                        {selectedSkill.is_bundled ? t("Bundled") : t("Workspace / Custom")}
                      </span>
                    </div>

                    {/* Action Buttons */}
                    <div className="flex items-center gap-1.5 shrink-0">
                      <Button
                        type="button"
                        onClick={() => handleUseInChat(selectedSkill)}
                        variant="primary"
                        size="sm"
                        leadingIcon={<Send className="h-3 w-3" />}
                        title={t("Use this skill in chat composer")}
                      >
                        {t("Use in Chat")}
                      </Button>

                      <IconButton
                        label={t("Copy /skill command")}
                        size="sm"
                        variant={
                          copiedKey === `cmd-${selectedSkill.name}` ? "primary" : "secondary"
                        }
                        type="button"
                        onClick={() =>
                          handleCopy(`cmd-${selectedSkill.name}`, `/skill ${selectedSkill.name}`)
                        }
                        title={t("Copy /skill command")}
                      >
                        {copiedKey === `cmd-${selectedSkill.name}` ? (
                          <Check className="h-3.5 w-3.5" />
                        ) : (
                          <Copy className="h-3.5 w-3.5" />
                        )}
                      </IconButton>
                    </div>
                  </div>

                  {/* Location & Directory */}
                  <div className="flex items-center gap-1.5 text-[11px] text-neutral-500 dark:text-neutral-400 font-mono truncate">
                    <span className="truncate">{selectedSkill.location}</span>
                  </div>

                  {/* Tab Switcher */}
                  <Tabs.Root
                    value={activeTab}
                    onValueChange={(value) => {
                      const nextTab = value as "doc" | "companions";
                      setActiveTab(nextTab);
                      if (nextTab === "companions" && !selectedCompanionPath) {
                        setSelectedCompanionPath(selectedSkill.companion_files[0] || null);
                      }
                    }}
                  >
                    <Tabs.List className="skills-detail-tabs" aria-label={t("Skill sections")}>
                      <Tabs.Trigger value="doc">
                        <BookOpen className="h-3.5 w-3.5" />
                        <span>SKILL.md</span>
                      </Tabs.Trigger>
                      {selectedSkill.companion_files &&
                        selectedSkill.companion_files.length > 0 && (
                          <Tabs.Trigger value="companions">
                            <Files className="h-3.5 w-3.5" />
                            <span>
                              {t("Companion Files")} ({selectedSkill.companion_files.length})
                            </span>
                          </Tabs.Trigger>
                        )}
                    </Tabs.List>
                  </Tabs.Root>
                </div>

                {/* Detail Body */}
                <div className="flex-1 p-4 overflow-y-auto max-h-[380px]">
                  {activeTab === "doc" ? (
                    <div className="prose prose-xs dark:prose-invert max-w-none">
                      <MarkdownRenderer content={selectedSkill.content || selectedSkill.document} />
                    </div>
                  ) : (
                    /* Companion Files Viewer */
                    <div className="space-y-3">
                      {/* Companion File Selector */}
                      <div className="flex flex-wrap gap-1.5">
                        {selectedSkill.companion_files.map((path) => (
                          <Button
                            type="button"
                            variant={selectedCompanionPath === path ? "primary" : "secondary"}
                            size="sm"
                            key={path}
                            onClick={() => setSelectedCompanionPath(path)}
                            className="font-mono"
                            leadingIcon={<FileCode className="h-3 w-3" />}
                          >
                            {path}
                          </Button>
                        ))}
                      </div>

                      {/* Companion File Content */}
                      <div className="rounded-lg border border-neutral-200 bg-neutral-50/70 p-3 dark:border-neutral-800 dark:bg-neutral-950/50">
                        <div className="flex items-center justify-between pb-2 mb-2 border-b border-neutral-200 dark:border-neutral-800 text-xs font-mono text-neutral-500">
                          <span>{selectedCompanionPath}</span>
                          {companionFileData?.content && (
                            <Button
                              type="button"
                              variant="ghost"
                              size="sm"
                              leadingIcon={
                                copiedKey === `file-${selectedCompanionPath}` ? (
                                  <Check className="h-3 w-3" />
                                ) : (
                                  <Copy className="h-3 w-3" />
                                )
                              }
                              onClick={() =>
                                handleCopy(
                                  `file-${selectedCompanionPath}`,
                                  companionFileData.content,
                                )
                              }
                            >
                              {t("Copy")}
                            </Button>
                          )}
                        </div>

                        {companionFileLoading ? (
                          <div className="flex items-center justify-center py-6 text-neutral-500">
                            <RefreshCw className="h-4 w-4 animate-spin mr-2" />
                            <span className="text-xs">{t("Loading file content...")}</span>
                          </div>
                        ) : companionFileError ? (
                          <p className="text-xs text-red-500 py-2">
                            {t("Failed to read companion file")}
                          </p>
                        ) : (
                          <pre className="font-mono text-xs text-neutral-800 dark:text-neutral-200 whitespace-pre-wrap overflow-x-auto leading-relaxed">
                            {companionFileData?.content || t("No content")}
                          </pre>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </>
            ) : (
              <div className="flex flex-col items-center justify-center h-full p-8 text-center text-neutral-400">
                <BookOpen className="h-8 w-8 mb-2 opacity-50" />
                <p className="text-xs">{t("Select a skill on the left to preview its content.")}</p>
              </div>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
