import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type TransitionEvent,
} from "react";

const EXPAND_DURATION_MS = 180;
const COLLAPSE_DURATION_MS = EXPAND_DURATION_MS;

type ExpandablePhase = "hidden" | "visible" | "hiding";
type BodyHeight = number | "auto";

interface Props {
  expanded: boolean;
  className: string;
  children: ReactNode;
}

function requestFrame(callback: () => void) {
  if (typeof window.requestAnimationFrame === "function") {
    return window.requestAnimationFrame(callback);
  }
  return window.setTimeout(callback, 0);
}

function cancelFrame(frame: number) {
  if (typeof window.cancelAnimationFrame === "function") {
    window.cancelAnimationFrame(frame);
  }
  window.clearTimeout(frame);
}

export function ExpandableBody({ expanded, className, children }: Props) {
  const [phase, setPhase] = useState<ExpandablePhase>(expanded ? "visible" : "hidden");
  const [height, setHeight] = useState<BodyHeight>(expanded ? "auto" : 0);
  const bodyRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const collapseTimer = useRef<number | null>(null);
  const transitionFrame = useRef<number | null>(null);
  const initialized = useRef(false);
  const previousExpanded = useRef(expanded);

  function clearCollapseTimer() {
    if (collapseTimer.current !== null) {
      window.clearTimeout(collapseTimer.current);
      collapseTimer.current = null;
    }
  }

  function clearTransitionFrame() {
    if (transitionFrame.current !== null) {
      cancelFrame(transitionFrame.current);
      transitionFrame.current = null;
    }
  }

  function contentHeight() {
    return Math.ceil(contentRef.current?.getBoundingClientRect().height ?? 0);
  }

  useLayoutEffect(() => {
    initialized.current = true;
    return () => {
      clearCollapseTimer();
      clearTransitionFrame();
    };
  }, []);

  useLayoutEffect(() => {
    clearTransitionFrame();

    if (expanded) {
      clearCollapseTimer();
      const wasExpanded = previousExpanded.current;
      previousExpanded.current = true;
      setPhase("visible");

      // A body that mounts because its virtualized row became visible should
      // appear at its natural height. Only a live collapse-to-expand change
      // gets an opening animation.
      if (!initialized.current || wasExpanded) {
        setHeight("auto");
        return;
      }

      setHeight(0);
      transitionFrame.current = requestFrame(() => {
        transitionFrame.current = null;
        setHeight(contentHeight());
      });
      return;
    }

    previousExpanded.current = false;
    if (phase !== "visible") {
      setHeight(0);
      return;
    }

    setPhase("hiding");
    setHeight(contentHeight());
    transitionFrame.current = requestFrame(() => {
      transitionFrame.current = null;
      setHeight(0);
    });
    clearCollapseTimer();
    collapseTimer.current = window.setTimeout(() => {
      collapseTimer.current = null;
      setPhase("hidden");
    }, COLLAPSE_DURATION_MS + 40);
  }, [expanded]);

  useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(() => {
      if (!expanded) return;
      setHeight((current) => (current === "auto" ? current : contentHeight()));
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [expanded]);

  if (!expanded && phase === "hidden") return null;

  const renderPhase = expanded ? "visible" : phase;

  function handleTransitionEnd(event: TransitionEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget || event.propertyName !== "height") return;

    if (renderPhase === "visible" && expanded) {
      setHeight("auto");
      return;
    }

    if (renderPhase === "hiding" && !expanded) {
      clearCollapseTimer();
      setHeight(0);
      setPhase("hidden");
    }
  }

  return (
    <div
      className={`${className} expandable-body expandable-body-${renderPhase}`}
      onTransitionEnd={handleTransitionEnd}
      ref={bodyRef}
      style={{ height: height === "auto" ? "auto" : `${height}px` }}
    >
      <div ref={contentRef}>{children}</div>
    </div>
  );
}
