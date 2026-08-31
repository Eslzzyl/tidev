import { useEffect, useRef } from "react";

export function useClickOutside(handler: () => void): React.RefObject<HTMLDivElement | null> {
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const handleClick = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node) || !ref.current || ref.current.contains(target)) return;
      if (
        event
          .composedPath()
          .some((node) => node instanceof Element && node.hasAttribute("data-ui-portal"))
      ) {
        return;
      }
      if (!event.defaultPrevented) handler();
    };

    document.addEventListener("click", handleClick, true);
    return () => document.removeEventListener("click", handleClick, true);
  }, [handler]);

  return ref;
}
