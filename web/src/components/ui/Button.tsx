import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";

import { cx } from "./utils";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ControlSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ControlSize;
  loading?: boolean;
  leadingIcon?: ReactNode;
  trailingIcon?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = "secondary",
    size = "md",
    loading = false,
    type = "button",
    leadingIcon,
    trailingIcon,
    className,
    children,
    disabled,
    ...props
  },
  ref,
) {
  return (
    <button
      ref={ref}
      className={cx("ui-button", className)}
      data-size={size}
      data-variant={variant}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      type={type}
      {...props}
    >
      {loading ? <span className="ui-spinner ui-spinner-inline" aria-hidden="true" /> : leadingIcon}
      <span className="ui-button-label">{children}</span>
      {!loading ? trailingIcon : null}
    </button>
  );
});

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  variant?: ButtonVariant;
  size?: ControlSize;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, variant = "ghost", size = "md", className, children, ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      className={cx("ui-icon-button", className)}
      data-size={size}
      data-variant={variant}
      aria-label={label}
      {...props}
      title={props.title ?? label}
    >
      {children}
    </button>
  );
});
