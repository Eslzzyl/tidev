import * as React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import * as TabsPrimitive from "@radix-ui/react-tabs";

import { cx } from "./utils";

export type SwitchProps = React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>;

export function Switch({ className, ...props }: SwitchProps) {
  return (
    <SwitchPrimitive.Root className={cx("ui-switch", className)} {...props}>
      <SwitchPrimitive.Thumb className="ui-switch-thumb" />
    </SwitchPrimitive.Root>
  );
}

export const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(function TabsList({ className, children, ...props }, forwardedRef) {
  const listRef = React.useRef<React.ElementRef<typeof TabsPrimitive.List>>(null);
  const [indicatorReady, setIndicatorReady] = React.useState(false);

  const setRefs = React.useCallback(
    (node: React.ElementRef<typeof TabsPrimitive.List> | null) => {
      listRef.current = node;

      if (typeof forwardedRef === "function") {
        forwardedRef(node);
      } else if (forwardedRef) {
        forwardedRef.current = node;
      }
    },
    [forwardedRef],
  );

  const updateIndicator = React.useCallback(() => {
    const list = listRef.current;
    const activeTrigger = list?.querySelector<HTMLElement>('[data-state="active"]');

    if (!list || !activeTrigger) {
      return;
    }

    const listRect = list.getBoundingClientRect();
    const triggerRect = activeTrigger.getBoundingClientRect();
    const listContentLeft = listRect.left + list.clientLeft;
    const listContentTop = listRect.top + list.clientTop;
    list.style.setProperty("--ui-tabs-indicator-x", `${triggerRect.left - listContentLeft}px`);
    list.style.setProperty("--ui-tabs-indicator-y", `${triggerRect.top - listContentTop}px`);
    list.style.setProperty("--ui-tabs-indicator-width", `${triggerRect.width}px`);
    list.style.setProperty("--ui-tabs-indicator-height", `${triggerRect.height}px`);
    setIndicatorReady(true);
  }, []);

  React.useLayoutEffect(() => {
    const list = listRef.current;

    if (!list || typeof window === "undefined") {
      return;
    }

    updateIndicator();

    const mutationObserver =
      typeof MutationObserver === "undefined" ? null : new MutationObserver(updateIndicator);
    mutationObserver?.observe(list, {
      subtree: true,
      attributes: true,
      attributeFilter: ["data-state"],
    });

    const resizeObserver =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateIndicator);
    resizeObserver?.observe(list);
    window.addEventListener("resize", updateIndicator);

    return () => {
      mutationObserver?.disconnect();
      resizeObserver?.disconnect();
      window.removeEventListener("resize", updateIndicator);
    };
  }, [updateIndicator]);

  return (
    <TabsPrimitive.List
      ref={setRefs}
      className={cx("ui-tabs-list", className)}
      data-indicator-ready={indicatorReady}
      {...props}
    >
      {children}
    </TabsPrimitive.List>
  );
});

export function TabsTrigger({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>) {
  return <TabsPrimitive.Trigger className={cx("ui-tabs-trigger", className)} {...props} />;
}

export function TabsContent({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content className={cx("ui-tabs-content", className)} {...props} />;
}

export const Tabs = {
  Root: TabsPrimitive.Root,
  List: TabsList,
  Trigger: TabsTrigger,
  Content: TabsContent,
};

export type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger";

export function Badge({
  tone = "neutral",
  className,
  ...props
}: React.HTMLAttributes<HTMLSpanElement> & { tone?: BadgeTone }) {
  return <span className={cx("ui-badge", className)} data-tone={tone} {...props} />;
}

export function Spinner({ className }: { className?: string }) {
  return <span className={cx("ui-spinner", className)} aria-label="Loading" role="status" />;
}
