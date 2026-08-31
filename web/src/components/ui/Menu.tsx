import { ChevronRight } from "lucide-react";
import * as MenuPrimitive from "@radix-ui/react-dropdown-menu";

import { cx } from "./utils";

export const MenuRoot = MenuPrimitive.Root;

export function MenuTrigger({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Trigger>) {
  return <MenuPrimitive.Trigger className={cx("ui-menu-trigger", className)} {...props} />;
}

export function MenuContent({
  className,
  sideOffset = 5,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Content>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        className={cx("ui-menu-content", className)}
        sideOffset={sideOffset}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

export function MenuItem({
  className,
  inset = false,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Item> & { inset?: boolean }) {
  return (
    <MenuPrimitive.Item
      className={cx("ui-menu-item", className)}
      data-inset={inset || undefined}
      {...props}
    />
  );
}

export function MenuLabel({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Label>) {
  return <MenuPrimitive.Label className={cx("ui-menu-label", className)} {...props} />;
}

export function MenuSeparator({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Separator>) {
  return <MenuPrimitive.Separator className={cx("ui-menu-separator", className)} {...props} />;
}

export function MenuSub(props: React.ComponentPropsWithoutRef<typeof MenuPrimitive.Sub>) {
  return <MenuPrimitive.Sub {...props} />;
}

export function MenuSubTrigger({
  className,
  children,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.SubTrigger>) {
  return (
    <MenuPrimitive.SubTrigger className={cx("ui-menu-item", className)} {...props}>
      {children}
      <ChevronRight className="ui-menu-sub-indicator" size={14} aria-hidden="true" />
    </MenuPrimitive.SubTrigger>
  );
}

export function MenuSubContent({
  className,
  sideOffset = 4,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.SubContent>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.SubContent
        className={cx("ui-menu-content", className)}
        sideOffset={sideOffset}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

export const Menu = {
  Root: MenuRoot,
  Trigger: MenuTrigger,
  Content: MenuContent,
  Item: MenuItem,
  Label: MenuLabel,
  Separator: MenuSeparator,
  Sub: MenuSub,
  SubTrigger: MenuSubTrigger,
  SubContent: MenuSubContent,
};
