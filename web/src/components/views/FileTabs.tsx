import { X, FileIcon } from "lucide-react";
import type { OpenFile } from "../../stores/useFileStore";
import { useTranslation } from "react-i18next";
import { IconButton, Tabs } from "../ui";

interface FileTabsProps {
  files: OpenFile[];
  activePath: string | null;
  onSelect: (path: string) => void;
  onClose: (path: string, e: React.MouseEvent) => void;
}

export function FileTabs({ files, activePath, onSelect, onClose }: FileTabsProps) {
  const { t } = useTranslation();
  if (files.length === 0) return null;

  return (
    <Tabs.Root value={activePath ?? undefined} onValueChange={onSelect} className="file-tabs-root">
      <Tabs.List className="file-tabs-list">
        {files.map((file) => (
          <Tabs.Item
            key={file.path}
            className={file.path === activePath ? "file-tab-item active" : "file-tab-item"}
          >
            <div className="file-tab-surface">
              <Tabs.Trigger value={file.path} className="file-tab">
                <FileIcon className="h-3 w-3 shrink-0 text-neutral-400" />
                <span className="max-w-[120px] truncate">
                  {file.path.split("/").pop() || file.path}
                </span>
                {file.isDirty && <span className="h-2 w-2 shrink-0 rounded-full bg-amber-400" />}
              </Tabs.Trigger>
              <IconButton
                label={t("Close {{path}}", { path: file.path })}
                size="sm"
                className="file-tab-close"
                onClick={(e) => onClose(file.path, e)}
              >
                <X className="h-3 w-3" />
              </IconButton>
            </div>
          </Tabs.Item>
        ))}
      </Tabs.List>
    </Tabs.Root>
  );
}
