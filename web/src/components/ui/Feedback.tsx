import type { HTMLAttributes, ReactNode } from "react";

import { cx } from "./utils";

export type AlertTone = "neutral" | "info" | "success" | "warning" | "danger";

export interface AlertProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  tone?: AlertTone;
  title?: ReactNode;
}

export function Alert({
  tone = "neutral",
  title,
  className,
  children,
  role = "status",
  ...props
}: AlertProps) {
  return (
    <div className={cx("ui-alert", className)} data-tone={tone} role={role} {...props}>
      {title ? <div className="ui-alert-title">{title}</div> : null}
      {children ? <div className="ui-alert-description">{children}</div> : null}
    </div>
  );
}
