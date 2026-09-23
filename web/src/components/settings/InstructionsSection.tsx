import { useEffect, useState } from "react";
import { FileText, RefreshCw, Save, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  useGlobalInstructions,
  useSaveGlobalInstructions,
  useDeleteGlobalInstructions,
} from "../../hooks/workspaceQueries";
import { getEffectiveTheme, useUIStore } from "../../stores/useUIStore";
import { CodeMirrorEditor } from "../ui/CodeMirrorEditor";
import { Button } from "../ui";
import { SettingsGroup, SettingsSectionHeader } from "./SettingsCommon";

export function InstructionsSection() {
  const { t } = useTranslation();
  const theme = useUIStore((state) => state.theme);
  const { data, isLoading, error, refetch } = useGlobalInstructions();
  const saveMutation = useSaveGlobalInstructions();
  const deleteMutation = useDeleteGlobalInstructions();
  const [draft, setDraft] = useState("");
  const [savedContent, setSavedContent] = useState("");

  useEffect(() => {
    if (data) {
      setDraft(data.content);
      setSavedContent(data.content);
    }
  }, [data]);

  const isDirty = draft !== savedContent;
  const isMutating = saveMutation.isPending || deleteMutation.isPending;

  const handleSave = async () => {
    const result = await saveMutation.mutateAsync(draft);
    setDraft(result.content);
    setSavedContent(result.content);
  };

  const handleDelete = async () => {
    const result = await deleteMutation.mutateAsync();
    setDraft(result.content);
    setSavedContent(result.content);
  };

  return (
    <section className="space-y-6">
      <SettingsSectionHeader
        title={
          <span className="flex items-center gap-2">
            <FileText className="h-4 w-4" />
            {t("Global Instructions")}
          </span>
        }
        description={t("Instructions applied at the beginning of new conversations")}
        action={
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void refetch()}
            disabled={isLoading || isMutating}
            leadingIcon={
              <RefreshCw className={isLoading ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
            }
          >
            {t("Reload")}
          </Button>
        }
      />

      <SettingsGroup
        title={data?.exists ? t("AGENTS.md") : t("File not created")}
        description={
          data
            ? t("Location: {{path}}", { path: data.path })
            : t("A global AGENTS.md file has not been created yet")
        }
      >
        {isLoading ? (
          <div className="flex items-center justify-center py-12 text-sm text-neutral-500">
            <RefreshCw className="mr-2 h-4 w-4 animate-spin" />
            {t("Loading global instructions...")}
          </div>
        ) : error ? (
          <div className="p-4 text-sm text-red-600 dark:text-red-400">
            {t("Failed to load global instructions")}
          </div>
        ) : (
          <>
            <div className="h-[min(55vh,32rem)] min-h-64 overflow-hidden border-b border-neutral-200/70 dark:border-neutral-800/70">
              <CodeMirrorEditor
                value={draft}
                onChange={setDraft}
                filePath="AGENTS.md"
                readOnly={isMutating}
                dark={getEffectiveTheme(theme) === "dark"}
                className="h-full"
              />
            </div>
            <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                {t("Changes apply to conversations that have not loaded this file yet")}
              </p>
              <div className="flex items-center gap-2">
                {data?.exists ? (
                  <Button
                    size="sm"
                    variant="danger"
                    onClick={() => void handleDelete()}
                    disabled={isMutating}
                    loading={deleteMutation.isPending}
                    leadingIcon={<Trash2 className="h-3.5 w-3.5" />}
                  >
                    {t("Delete")}
                  </Button>
                ) : null}
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => void handleSave()}
                  disabled={!isDirty || isMutating}
                  loading={saveMutation.isPending}
                  leadingIcon={<Save className="h-3.5 w-3.5" />}
                >
                  {t("Save")}
                </Button>
              </div>
            </div>
            {saveMutation.error || deleteMutation.error ? (
              <div className="px-4 pb-3 text-xs text-red-600 dark:text-red-400">
                {t("Failed to save global instructions")}
              </div>
            ) : null}
          </>
        )}
      </SettingsGroup>
    </section>
  );
}
