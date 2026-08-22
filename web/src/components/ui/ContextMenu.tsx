import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

export interface ContextMenuItem {
  type?: "item" | "separator";
  label?: string;
  icon?: React.ReactNode;
  shortcut?: string;
  translated?: boolean;
  disabled?: boolean;
  danger?: boolean;
  onClick?: () => void;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const { t } = useTranslation();
  const menuRef = useRef<HTMLDivElement>(null);

  // Close on click outside or Escape
  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };

    // Delay adding listener to avoid the current right-click event
    const timer = setTimeout(() => {
      document.addEventListener("mousedown", handleClick);
      document.addEventListener("keydown", handleKeyDown);
    }, 0);

    return () => {
      clearTimeout(timer);
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  // Adjust position to keep menu in viewport
  const adjustedX = Math.min(x, window.innerWidth - 200);
  const adjustedY = Math.min(y, window.innerHeight - items.length * 32 - 16);

  return (
    <div
      ref={menuRef}
      className="fixed z-[9999] min-w-[160px] rounded-lg border border-neutral-200 bg-white py-1 shadow-lg dark:border-neutral-700 dark:bg-neutral-900"
      style={{ left: adjustedX, top: adjustedY }}
    >
      {items.map((item, i) =>
        item.type === "separator" ? (
          <div key={i} className="my-1 border-t border-neutral-200 dark:border-neutral-700" />
        ) : (
          <button
            key={i}
            onClick={() => {
              if (!item.disabled) {
                item.onClick?.();
                onClose();
              }
            }}
            disabled={item.disabled}
            className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs ${
              item.danger
                ? "text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-950"
                : "text-neutral-700 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800"
            } ${item.disabled ? "cursor-not-allowed opacity-40" : "cursor-pointer"}`}
          >
            {item.icon && <span className="h-3.5 w-3.5 shrink-0">{item.icon}</span>}
            <span className="flex-1">
              {item.label ? (item.translated === false ? item.label : t(item.label)) : null}
            </span>
            {item.shortcut && <span className="text-[10px] text-neutral-400">{item.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}
