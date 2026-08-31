import { createContext, useCallback, useContext, useState } from "react";
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
        data-ui-portal="true"
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

type MenuSubProps = React.ComponentPropsWithoutRef<typeof MenuPrimitive.Sub> & {
  instant?: boolean;
};

interface InstantMenuSubContextValue {
  open: boolean;
  openImmediately: () => void;
}

const InstantMenuSubContext = createContext<InstantMenuSubContextValue | null>(null);

export function MenuSub({
  instant = false,
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  children,
  ...props
}: MenuSubProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const isControlled = openProp !== undefined;
  const open = isControlled ? openProp : uncontrolledOpen;
  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!isControlled) setUncontrolledOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [isControlled, onOpenChange],
  );
  const openImmediately = useCallback(() => {
    if (!open) handleOpenChange(true);
  }, [handleOpenChange, open]);

  if (!instant) {
    return (
      <MenuPrimitive.Sub
        {...props}
        open={openProp}
        defaultOpen={defaultOpen}
        onOpenChange={onOpenChange}
      >
        {children}
      </MenuPrimitive.Sub>
    );
  }

  return (
    <InstantMenuSubContext.Provider value={{ open, openImmediately }}>
      <MenuPrimitive.Sub {...props} open={open} onOpenChange={handleOpenChange}>
        {children}
      </MenuPrimitive.Sub>
    </InstantMenuSubContext.Provider>
  );
}

export function MenuSubTrigger({
  className,
  children,
  onPointerMove,
  ...props
}: React.ComponentPropsWithoutRef<typeof MenuPrimitive.SubTrigger>) {
  const instantSub = useContext(InstantMenuSubContext);

  return (
    <MenuPrimitive.SubTrigger
      className={cx("ui-menu-item", className)}
      onPointerMove={(event) => {
        onPointerMove?.(event);
        if (
          event.defaultPrevented ||
          event.pointerType !== "mouse" ||
          !instantSub ||
          instantSub.open
        ) {
          return;
        }
        instantSub.openImmediately();
        event.preventDefault();
      }}
      {...props}
    >
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
        data-ui-portal="true"
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
