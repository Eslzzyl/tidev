import { useLayoutEffect, useRef, useState, type ReactNode, type TransitionEvent } from "react";

const EXPAND_DURATION_MS = 180;
const COLLAPSE_DURATION_MS = EXPAND_DURATION_MS;

type ExpandablePhase = "hidden" | "visible" | "hiding";

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
  const [openTarget, setOpenTarget] = useState(expanded);
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
        setOpenTarget(true);
        return;
      }

      setOpenTarget(false);
      transitionFrame.current = requestFrame(() => {
        transitionFrame.current = null;
        setOpenTarget(true);
      });
      return;
    }

    previousExpanded.current = false;
    if (phase !== "visible") {
      setOpenTarget(false);
      return;
    }

    setPhase("hiding");
    setOpenTarget(true);
    transitionFrame.current = requestFrame(() => {
      transitionFrame.current = null;
      setOpenTarget(false);
    });
    clearCollapseTimer();
    collapseTimer.current = window.setTimeout(() => {
      collapseTimer.current = null;
      setPhase("hidden");
    }, COLLAPSE_DURATION_MS + 40);
  }, [expanded]);

  if (!expanded && phase === "hidden") return null;

  const renderPhase = expanded ? "visible" : phase;

  function handleTransitionEnd(event: TransitionEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget || event.propertyName !== "grid-template-rows") return;

    if (renderPhase === "hiding" && !expanded) {
      clearCollapseTimer();
      setOpenTarget(false);
      setPhase("hidden");
    }
  }

  return (
    <div
      className={`${className} expandable-body expandable-body-${renderPhase}`}
      onTransitionEnd={handleTransitionEnd}
      style={{ gridTemplateRows: openTarget ? "1fr" : "0fr" }}
    >
      <div>{children}</div>
    </div>
  );
}
