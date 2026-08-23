import type { ReactNode } from "react";

interface Props {
  children?: ReactNode;
  label?: string;
  active?: boolean;
  row?: boolean;
}

/**
 * A small inline activity indicator whose waves sit on top of the label.
 * Keeping the effect in CSS makes it cheap to render for every live row.
 */
export function ActivityRipple({ children, label, active = true, row = false }: Props) {
  if (!active) return <>{children}</>;

  return (
    <span
      className={`activity-ripple${row ? " activity-ripple-row" : ""}`}
      role="status"
      aria-label={label}
    >
      <span className="activity-ripple-content">{children ?? ""}</span>
    </span>
  );
}
