import { useId, type ReactNode } from "react";
import { Check, ChevronDown, ChevronUp } from "lucide-react";
import * as SelectPrimitive from "@radix-ui/react-select";

import type { ControlSize } from "./Button";
import { cx } from "./utils";

const EMPTY_VALUE = "__tidev_select_empty__";

export interface SelectOption {
  value: string;
  label: ReactNode;
  disabled?: boolean;
}

export interface SelectGroup {
  label: ReactNode;
  options: SelectOption[];
}

export interface SelectProps {
  id?: string;
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  options?: SelectOption[];
  groups?: SelectGroup[];
  label?: ReactNode;
  description?: ReactNode;
  error?: ReactNode;
  placeholder?: ReactNode;
  ariaLabel?: string;
  name?: string;
  required?: boolean;
  disabled?: boolean;
  size?: ControlSize;
  className?: string;
  triggerClassName?: string;
}

function encodeValue(value: string | undefined): string | undefined {
  return value === "" ? EMPTY_VALUE : value;
}

function decodeValue(value: string): string {
  return value === EMPTY_VALUE ? "" : value;
}

export function Select({
  id,
  value,
  defaultValue,
  onValueChange,
  options = [],
  groups = [],
  label,
  description,
  error,
  placeholder = "Select an option",
  ariaLabel,
  name,
  required,
  disabled,
  size = "md",
  className,
  triggerClassName,
}: SelectProps) {
  const generatedId = useId();
  const triggerId = id ?? `tidev-select-${generatedId.replace(/:/g, "")}`;
  const allOptions = groups.length > 0 ? groups.flatMap((group) => group.options) : options;
  const normalizedValue = encodeValue(value);
  const normalizedDefaultValue = encodeValue(defaultValue);

  return (
    <div className={cx("ui-field", className)}>
      {label ? (
        <label className="ui-field-label" htmlFor={triggerId}>
          {label}
        </label>
      ) : null}
      {description ? <p className="ui-field-description">{description}</p> : null}
      <SelectPrimitive.Root
        value={normalizedValue}
        defaultValue={normalizedDefaultValue}
        onValueChange={(nextValue) => onValueChange?.(decodeValue(nextValue))}
        name={name}
        required={required}
        disabled={disabled}
      >
        <SelectPrimitive.Trigger
          id={triggerId}
          className={cx("ui-control ui-select-trigger", triggerClassName)}
          data-size={size}
          data-invalid={error ? "true" : undefined}
          aria-invalid={error ? true : undefined}
          aria-label={label ? undefined : ariaLabel}
        >
          <SelectPrimitive.Value placeholder={placeholder} />
          <SelectPrimitive.Icon asChild>
            <ChevronDown className="ui-select-chevron" aria-hidden="true" />
          </SelectPrimitive.Icon>
        </SelectPrimitive.Trigger>
        <SelectPrimitive.Portal>
          <SelectPrimitive.Content className="ui-select-content" position="popper" sideOffset={4}>
            <SelectPrimitive.ScrollUpButton className="ui-select-scroll-button">
              <ChevronUp size={14} aria-hidden="true" />
            </SelectPrimitive.ScrollUpButton>
            <SelectPrimitive.Viewport className="ui-select-viewport">
              {groups.length > 0
                ? groups.map((group) => (
                    <SelectPrimitive.Group key={String(group.label)}>
                      <SelectPrimitive.Label className="ui-select-group-label">
                        {group.label}
                      </SelectPrimitive.Label>
                      {group.options.map((option) => (
                        <SelectOptionItem key={option.value} option={option} />
                      ))}
                    </SelectPrimitive.Group>
                  ))
                : allOptions.map((option) => (
                    <SelectOptionItem key={option.value} option={option} />
                  ))}
            </SelectPrimitive.Viewport>
            <SelectPrimitive.ScrollDownButton className="ui-select-scroll-button">
              <ChevronDown size={14} aria-hidden="true" />
            </SelectPrimitive.ScrollDownButton>
          </SelectPrimitive.Content>
        </SelectPrimitive.Portal>
      </SelectPrimitive.Root>
      {error ? (
        <p className="ui-field-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function SelectOptionItem({ option }: { option: SelectOption }) {
  return (
    <SelectPrimitive.Item
      value={encodeValue(option.value) ?? EMPTY_VALUE}
      disabled={option.disabled}
      className="ui-select-item"
    >
      <SelectPrimitive.ItemText>{option.label}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator className="ui-select-item-indicator">
        <Check size={14} aria-hidden="true" />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
}
