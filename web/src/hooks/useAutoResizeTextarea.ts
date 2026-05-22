import { useEffect, type RefObject } from "react";

/**
 * Automatically resizes a textarea to fit its content, up to a maximum height.
 * Once the content exceeds the max height, overflow-y becomes scrollable.
 */
export function useAutoResizeTextarea(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxHeight: number = 200,
) {
  useEffect(() => {
    const textarea = ref.current;
    if (!textarea || !(textarea instanceof HTMLTextAreaElement)) return;

    // Reset to auto height so scrollHeight reflects the true content height
    textarea.style.height = "auto";

    const scrollHeight = textarea.scrollHeight;
    const newHeight = Math.min(scrollHeight, maxHeight);

    textarea.style.height = `${newHeight}px`;

    // Show scrollbar only when content exceeds the max height
    textarea.style.overflowY = scrollHeight > maxHeight ? "auto" : "hidden";
  }, [ref, value, maxHeight]);
}
