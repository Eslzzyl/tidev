import * as CollapsiblePrimitive from "@radix-ui/react-collapsible";

import { cx } from "./utils";

export function CollapsibleRoot({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof CollapsiblePrimitive.Root>) {
  return <CollapsiblePrimitive.Root className={cx("ui-collapsible", className)} {...props} />;
}

export function CollapsibleTrigger({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof CollapsiblePrimitive.Trigger>) {
  return (
    <CollapsiblePrimitive.Trigger className={cx("ui-collapsible-trigger", className)} {...props} />
  );
}

export function CollapsibleContent({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof CollapsiblePrimitive.Content>) {
  return (
    <CollapsiblePrimitive.Content className={cx("ui-collapsible-content", className)} {...props} />
  );
}

export const Collapsible = {
  Root: CollapsibleRoot,
  Trigger: CollapsibleTrigger,
  Content: CollapsibleContent,
};
