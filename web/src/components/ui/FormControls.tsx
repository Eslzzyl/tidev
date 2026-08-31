import { forwardRef, type InputHTMLAttributes, type TextareaHTMLAttributes } from "react";

import type { ControlSize } from "./Button";
import { cx } from "./utils";

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  size?: ControlSize;
  invalid?: boolean;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { size = "md", invalid = false, className, ...props },
  ref,
) {
  return (
    <input
      ref={ref}
      className={cx("ui-control ui-input", className)}
      data-size={size}
      data-invalid={invalid || undefined}
      aria-invalid={invalid || undefined}
      {...props}
    />
  );
});

export interface TextareaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  size?: ControlSize;
  invalid?: boolean;
}

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(function Textarea(
  { size = "md", invalid = false, className, ...props },
  ref,
) {
  return (
    <textarea
      ref={ref}
      className={cx("ui-control ui-textarea", className)}
      data-size={size}
      data-invalid={invalid || undefined}
      aria-invalid={invalid || undefined}
      {...props}
    />
  );
});

export interface FieldProps {
  label?: React.ReactNode;
  description?: React.ReactNode;
  error?: React.ReactNode;
  required?: boolean;
  htmlFor?: string;
  className?: string;
  children: React.ReactNode;
}

export function Field({
  label,
  description,
  error,
  required = false,
  htmlFor,
  className,
  children,
}: FieldProps) {
  return (
    <div className={cx("ui-field", className)}>
      {label ? (
        <label className="ui-field-label" htmlFor={htmlFor}>
          <span>{label}</span>
          {required ? <span className="ui-field-required">*</span> : null}
        </label>
      ) : null}
      {description ? <p className="ui-field-description">{description}</p> : null}
      {children}
      {error ? (
        <p className="ui-field-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
