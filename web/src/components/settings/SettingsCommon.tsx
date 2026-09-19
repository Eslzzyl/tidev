import React from "react";
import { Switch, Select, Input } from "../ui";
import { cx } from "../ui/utils";

/* -------------------------------------------------------------------------- */
/* Section Header                                                             */
/* -------------------------------------------------------------------------- */

export interface SettingsSectionHeaderProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}

export function SettingsSectionHeader({
  title,
  description,
  action,
  className,
}: SettingsSectionHeaderProps) {
  return (
    <div className={cx("flex items-start justify-between gap-4 mb-5", className)}>
      <div className="min-w-0">
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100 tracking-tight">
          {title}
        </h2>
        {description ? (
          <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400 leading-relaxed">
            {description}
          </p>
        ) : null}
      </div>
      {action ? <div className="shrink-0 pt-0.5">{action}</div> : null}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Group Container                                                            */
/* -------------------------------------------------------------------------- */

export interface SettingsGroupProps {
  title?: React.ReactNode;
  description?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  containerClassName?: string;
}

export function SettingsGroup({
  title,
  description,
  children,
  className,
  containerClassName,
}: SettingsGroupProps) {
  return (
    <div className={cx("space-y-2", className)}>
      {title ? (
        <div className="px-0.5">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
            {title}
          </h3>
          {description ? (
            <p className="mt-0.5 text-[11px] text-neutral-400 dark:text-neutral-500 leading-relaxed">
              {description}
            </p>
          ) : null}
        </div>
      ) : null}
      <div
        className={cx(
          "rounded-xl border border-neutral-200/80 bg-neutral-50/50 divide-y divide-neutral-200/60",
          "dark:border-neutral-800/80 dark:bg-neutral-800/30 dark:divide-neutral-800/60",
          "overflow-hidden transition-colors",
          containerClassName,
        )}
      >
        {children}
      </div>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Base Row                                                                   */
/* -------------------------------------------------------------------------- */

export interface SettingsRowProps {
  label: React.ReactNode;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  control?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
  onClick?: () => void;
  disabled?: boolean;
}

export function SettingsRow({
  label,
  description,
  badge,
  control,
  children,
  className,
  onClick,
  disabled,
}: SettingsRowProps) {
  return (
    <div
      className={cx(
        "flex items-center justify-between gap-4 px-4 py-3 sm:py-3.5 min-h-[56px] transition-colors",
        onClick &&
          !disabled &&
          "cursor-pointer select-none hover:bg-neutral-100/50 dark:hover:bg-neutral-800/50",
        disabled && "opacity-60 cursor-not-allowed",
        className,
      )}
      onClick={!disabled ? onClick : undefined}
    >
      <div className="flex-1 min-w-0 pr-2">
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
            {label}
          </span>
          {badge}
        </div>
        {description ? (
          <p className="mt-0.5 text-[11px] text-neutral-500 dark:text-neutral-400 leading-relaxed">
            {description}
          </p>
        ) : null}
      </div>
      {control ? (
        <div
          className="shrink-0 flex items-center justify-end"
          onClick={(e) => e.stopPropagation()}
        >
          {control}
        </div>
      ) : null}
      {children}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Switch Row                                                                 */
/* -------------------------------------------------------------------------- */

export interface SettingsSwitchRowProps {
  label: React.ReactNode;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
  className?: string;
}

export function SettingsSwitchRow({
  label,
  description,
  badge,
  checked,
  onCheckedChange,
  disabled = false,
  ariaLabel,
  className,
}: SettingsSwitchRowProps) {
  return (
    <label
      className={cx(
        "flex items-center justify-between gap-4 px-4 py-3 sm:py-3.5 min-h-[56px] transition-colors cursor-pointer select-none",
        "hover:bg-neutral-100/40 dark:hover:bg-neutral-800/40",
        disabled && "opacity-60 cursor-not-allowed",
        className,
      )}
    >
      <div className="flex-1 min-w-0 pr-2">
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
            {label}
          </span>
          {badge}
        </div>
        {description ? (
          <p className="mt-0.5 text-[11px] text-neutral-500 dark:text-neutral-400 leading-relaxed">
            {description}
          </p>
        ) : null}
      </div>
      <div className="shrink-0 flex items-center justify-end">
        <Switch
          checked={checked}
          onCheckedChange={onCheckedChange}
          disabled={disabled}
          aria-label={ariaLabel || (typeof label === "string" ? label : undefined)}
        />
      </div>
    </label>
  );
}

/* -------------------------------------------------------------------------- */
/* Select Row                                                                 */
/* -------------------------------------------------------------------------- */

export interface SettingsSelectRowProps {
  label: React.ReactNode;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  value: string;
  onValueChange: (value: string) => void;
  options?: { value: string; label: string }[];
  groups?: { label: string; options: { value: string; label: string }[] }[];
  disabled?: boolean;
  placeholder?: string;
  ariaLabel?: string;
  selectClassName?: string;
  className?: string;
}

export function SettingsSelectRow({
  label,
  description,
  badge,
  value,
  onValueChange,
  options,
  groups,
  disabled = false,
  placeholder,
  ariaLabel,
  selectClassName,
  className,
}: SettingsSelectRowProps) {
  return (
    <SettingsRow
      label={label}
      description={description}
      badge={badge}
      className={className}
      control={
        <Select
          value={value}
          onValueChange={onValueChange}
          options={options}
          groups={groups}
          disabled={disabled}
          placeholder={placeholder}
          ariaLabel={ariaLabel || (typeof label === "string" ? label : undefined)}
          className={cx("shrink-0 min-w-[140px] max-w-[280px]", selectClassName)}
        />
      }
    />
  );
}

/* -------------------------------------------------------------------------- */
/* Input Row                                                                  */
/* -------------------------------------------------------------------------- */

export interface SettingsInputRowProps {
  label: React.ReactNode;
  description?: React.ReactNode;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  type?: string;
  mono?: boolean;
  disabled?: boolean;
  inputClassName?: string;
  className?: string;
}

export function SettingsInputRow({
  label,
  description,
  value,
  onChange,
  placeholder,
  type = "text",
  mono = false,
  disabled = false,
  inputClassName,
  className,
}: SettingsInputRowProps) {
  return (
    <SettingsRow
      label={label}
      description={description}
      className={className}
      control={
        <Input
          type={type}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className={cx("w-56 sm:w-72 text-xs", mono && "font-mono", inputClassName)}
        />
      }
    />
  );
}

/* -------------------------------------------------------------------------- */
/* Action Row                                                                 */
/* -------------------------------------------------------------------------- */

export interface SettingsActionRowProps {
  label: React.ReactNode;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  action: React.ReactNode;
  className?: string;
  disabled?: boolean;
}

export function SettingsActionRow({
  label,
  description,
  badge,
  action,
  className,
  disabled,
}: SettingsActionRowProps) {
  return (
    <SettingsRow
      label={label}
      description={description}
      badge={badge}
      disabled={disabled}
      className={className}
      control={action}
    />
  );
}
