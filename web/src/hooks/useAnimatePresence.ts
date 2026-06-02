import { useState, useEffect, useRef } from "react";

/**
 * Lightweight mount/unmount transition manager.
 *
 * Handles the two-phase lifecycle needed for CSS exit animations:
 *   1. When `show` becomes true → immediately mount, then next-frame set visible
 *   2. When `show` becomes false → immediately set hidden, wait `exitDuration` ms, then unmount
 *
 * @param show          Whether the component should logically be visible
 * @param exitDuration  How long the exit CSS animation lasts (ms). Default 200.
 * @returns             `{ mounted, visible, stage }`
 *                      - `mounted`:  render the element when true
 *                      - `visible`:  apply "visible" CSS state (opacity:1, etc.)
 *                      - `stage`:    "entering" | "entered" | "exiting"
 */
export function useAnimatePresence(
  show: boolean,
  exitDuration = 200,
): {
  mounted: boolean;
  visible: boolean;
  stage: "entering" | "entered" | "exiting";
} {
  const [mounted, setMounted] = useState(show);
  const [visible, setVisible] = useState(show);
  const [stage, setStage] = useState<"entering" | "entered" | "exiting">(
    show ? "entered" : "exiting",
  );
  const prevShow = useRef(show);

  useEffect(() => {
    if (show === prevShow.current) return;
    prevShow.current = show;

    if (show) {
      // Mount immediately, then in the next frame trigger entrance animation
      const raf = requestAnimationFrame(() => {
        setMounted(true);
        setStage("entering");
        // Next frame: trigger entrance animation
        requestAnimationFrame(() => {
          setVisible(true);
          // After entrance animation completes, mark as entered
          // (we use a small timeout matching typical entrance duration)
          setTimeout(() => setStage("entered"), 250);
        });
      });
      return () => cancelAnimationFrame(raf);
    } else {
      // Start exit
      const raf = requestAnimationFrame(() => {
        setStage("exiting");
        setVisible(false);
      });
      const timer = setTimeout(() => {
        setMounted(false);
      }, exitDuration);
      return () => {
        cancelAnimationFrame(raf);
        clearTimeout(timer);
      };
    }
  }, [show, exitDuration]);

  return { mounted, visible, stage };
}
