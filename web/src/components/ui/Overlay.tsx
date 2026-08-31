import { X } from "lucide-react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as PopoverPrimitive from "@radix-ui/react-popover";

import { cx } from "./utils";

export const PopoverRoot = PopoverPrimitive.Root;

export function PopoverTrigger({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger className={cx("ui-popover-trigger", className)} {...props} />;
}

export function PopoverContent({
  className,
  sideOffset = 8,
  ...props
}: React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        data-ui-portal="true"
        className={cx("ui-popover-content", className)}
        sideOffset={sideOffset}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}

export const Popover = {
  Root: PopoverRoot,
  Trigger: PopoverTrigger,
  Content: PopoverContent,
  Close: PopoverPrimitive.Close,
  Anchor: PopoverPrimitive.Anchor,
};

export const DialogRoot = DialogPrimitive.Root;

export function DialogTrigger({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger className={cx("ui-dialog-trigger", className)} {...props} />;
}

export function DialogContent({
  className,
  children,
  showClose = true,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> & { showClose?: boolean }) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay data-ui-portal="true" className="ui-dialog-overlay" />
      <DialogPrimitive.Content
        data-ui-portal="true"
        className={cx("ui-dialog-content", className)}
        {...props}
      >
        {children}
        {showClose ? (
          <DialogPrimitive.Close className="ui-dialog-close" aria-label="Close">
            <X size={15} aria-hidden="true" />
          </DialogPrimitive.Close>
        ) : null}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}

export function DialogHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cx("ui-dialog-header", className)} {...props} />;
}

export function DialogTitle({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>) {
  return <DialogPrimitive.Title className={cx("ui-dialog-title", className)} {...props} />;
}

export function DialogDescription({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description className={cx("ui-dialog-description", className)} {...props} />
  );
}

export function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cx("ui-dialog-footer", className)} {...props} />;
}

export const Dialog = {
  Root: DialogRoot,
  Trigger: DialogTrigger,
  Content: DialogContent,
  Header: DialogHeader,
  Title: DialogTitle,
  Description: DialogDescription,
  Footer: DialogFooter,
  Close: DialogPrimitive.Close,
};
