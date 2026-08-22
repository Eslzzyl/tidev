import { useCallback, useEffect, useState } from "react";
import { useTerminalStore } from "../../stores/useTerminalStore";
import { useTranslation } from "react-i18next";

interface TerminalTouchKeyboardProps {
  tabId: string;
  sendInput: (tabId: string, data: string) => void;
  isDark: boolean;
}

interface KeyDef {
  label: string;
  data: string;
  /** If true, this key toggles Ctrl latch instead of sending data */
  ctrlToggle?: boolean;
}

const KEY_ROWS: KeyDef[][] = [
  [
    { label: "Tab", data: "\t" },
    { label: "Esc", data: "\x1b" },
    { label: "Ctrl", data: "", ctrlToggle: true },
    { label: "↑", data: "\x1b[A" },
    { label: "/", data: "/" },
  ],
  [
    { label: "-", data: "-" },
    { label: ".", data: "." },
    { label: "←", data: "\x1b[D" },
    { label: "↓", data: "\x1b[B" },
    { label: "→", data: "\x1b[C" },
  ],
];

export function TerminalTouchKeyboard({ tabId, sendInput, isDark }: TerminalTouchKeyboardProps) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const [systemKbHeight, setSystemKbHeight] = useState(0);
  const ctrlLatch = useTerminalStore((s) => s.ctrlLatch);
  const setCtrlLatch = useTerminalStore((s) => s.setCtrlLatch);

  // Detect touch-capable device
  useEffect(() => {
    const hasTouch = "ontouchstart" in window || navigator.maxTouchPoints > 0;
    setVisible(hasTouch);
  }, []);

  // Track system keyboard via Visual Viewport API
  useEffect(() => {
    if (!visible) return;
    const vv = window.visualViewport;
    if (!vv) return;

    const update = () => {
      setSystemKbHeight(Math.max(0, window.innerHeight - vv.height));
    };

    vv.addEventListener("resize", update);
    update();
    return () => vv.removeEventListener("resize", update);
  }, [visible]);

  const handleClick = useCallback(
    (e: React.MouseEvent, key: KeyDef) => {
      e.preventDefault();
      if (key.ctrlToggle) {
        setCtrlLatch(!ctrlLatch);
        return;
      }
      // If Ctrl is latched and a letter is pressed, send Ctrl+letter
      if (ctrlLatch && key.data.length === 1) {
        const code = key.data.charCodeAt(0);
        if (code >= 97 && code <= 122) {
          sendInput(tabId, String.fromCharCode(code - 96));
          setCtrlLatch(false);
          return;
        }
        // Uppercase: also fold to Ctrl+letter (mobile keyboards sometimes auto-capitalize)
        if (code >= 65 && code <= 90) {
          sendInput(tabId, String.fromCharCode(code - 64));
          setCtrlLatch(false);
          return;
        }
      }
      sendInput(tabId, key.data);
    },
    [ctrlLatch, sendInput, setCtrlLatch, tabId],
  );

  if (!visible) return null;

  return (
    <div
      style={{
        position: "fixed",
        bottom: systemKbHeight,
        left: 0,
        right: 0,
        zIndex: 40,
      }}
      className={`flex animate-fade-in flex-col gap-px border-t px-1 pb-1 pt-0.5 ${
        isDark
          ? "border-neutral-800 bg-neutral-950/85 backdrop-blur-sm"
          : "border-neutral-200 bg-white/85 backdrop-blur-sm"
      }`}
    >
      {KEY_ROWS.map((row, ri) => (
        <div key={ri} className="flex gap-px">
          {row.map((key) => {
            const isCtrl = key.ctrlToggle;
            const active = isCtrl && ctrlLatch;
            return (
              <button
                key={key.label}
                type="button"
                tabIndex={-1}
                onClick={(e) => handleClick(e, key)}
                className={`flex-1 cursor-pointer select-none rounded px-1 py-1.5 text-center text-xs font-medium leading-none transition-colors active:scale-95 ${
                  active
                    ? "bg-blue-600 text-white shadow-sm"
                    : isDark
                      ? "bg-neutral-800 text-neutral-200 hover:bg-neutral-700 active:bg-neutral-600"
                      : "bg-neutral-100 text-neutral-700 hover:bg-neutral-200 active:bg-neutral-300"
                }`}
              >
                {key.label === "Tab" || key.label === "Esc" || key.label === "Ctrl"
                  ? t(key.label)
                  : key.label}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}
