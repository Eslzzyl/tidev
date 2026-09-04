import { useTranslation } from "react-i18next";

import { useSubagentConfig } from "../../hooks/workspaceQueries";

export function SubagentStatusIndicator() {
  const { t } = useTranslation();
  const { data: subagentConfig } = useSubagentConfig();

  if (subagentConfig?.enabled !== false) return null;

  return (
    <span className="composer-subagent-disabled" aria-label={t("Subagent is disabled")}>
      <s>{t("Subagent")}</s>
    </span>
  );
}
