import { useState, useEffect, useCallback } from "react";
import { X, Palette, Type, Keyboard, Lock, Info } from "lucide-react";
import { useUIStore } from "../../stores/useUIStore";
import { useClickOutside } from "../../hooks/useClickOutside";
import { AppearanceSection } from "./AppearanceSection";
import { EditorSection } from "./EditorSection";
import { InteractionSection } from "./InteractionSection";
import { SecuritySection } from "./SecuritySection";
import { AboutSection } from "./AboutSection";

type CategoryId = "appearance" | "editor" | "interaction" | "security" | "about";

interface Category {
  id: CategoryId;
  label: string;
  icon: React.ReactNode;
}

const categories: Category[] = [
  { id: "appearance", label: "Appearance", icon: <Palette className="h-4 w-4" /> },
  { id: "editor", label: "Editor", icon: <Type className="h-4 w-4" /> },
  { id: "interaction", label: "Interaction", icon: <Keyboard className="h-4 w-4" /> },
  { id: "security", label: "Security", icon: <Lock className="h-4 w-4" /> },
  { id: "about", label: "About", icon: <Info className="h-4 w-4" /> },
];

export function SettingsPanel() {
  const settingsPanelOpen = useUIStore((s) => s.settingsPanelOpen);
  const closeSettingsPanel = useUIStore((s) => s.closeSettingsPanel);
  const [activeCategory, setActiveCategory] = useState<CategoryId>("appearance");

  const panelRef = useClickOutside(closeSettingsPanel);

  // Close on Escape
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        closeSettingsPanel();
      }
    },
    [closeSettingsPanel],
  );

  useEffect(() => {
    if (settingsPanelOpen) {
      document.addEventListener("keydown", handleKeyDown);
      // Prevent body scroll while panel is open
      document.body.style.overflow = "hidden";
    }
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = "";
    };
  }, [settingsPanelOpen, handleKeyDown]);

  if (!settingsPanelOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center motion-safe:animate-fade-in bg-black/50 p-4">
      <div
        ref={panelRef}
        className="motion-safe:animate-scale-fade flex max-h-[80vh] w-full max-w-[680px] flex-col overflow-hidden rounded-xl bg-white shadow-2xl dark:bg-neutral-900"
      >
        {/* Header */}
        <div className="flex shrink-0 items-center justify-between border-b border-neutral-200 px-5 py-3 dark:border-neutral-800">
          <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
            Settings
          </h2>
          <button
            onClick={closeSettingsPanel}
            className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Close settings"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body: sidebar + content */}
        <div className="flex min-h-0 flex-1">
          {/* Sidebar */}
          <nav className="w-36 shrink-0 border-r border-neutral-200 p-2 dark:border-neutral-800">
            {categories.map((cat) => (
              <button
                key={cat.id}
                onClick={() => setActiveCategory(cat.id)}
                className={`flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors ${
                  activeCategory === cat.id
                    ? "bg-neutral-100 font-medium text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100"
                    : "text-neutral-500 hover:bg-neutral-50 hover:text-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800/50 dark:hover:text-neutral-300"
                }`}
              >
                {cat.icon}
                <span>{cat.label}</span>
              </button>
            ))}
          </nav>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-5">
            {activeCategory === "appearance" && <AppearanceSection />}
            {activeCategory === "editor" && <EditorSection />}
            {activeCategory === "interaction" && <InteractionSection />}
            {activeCategory === "security" && <SecuritySection />}
            {activeCategory === "about" && <AboutSection />}
          </div>
        </div>

        {/* Footer */}
        <div className="flex shrink-0 items-center justify-between border-t border-neutral-200 px-5 py-3 dark:border-neutral-800">
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            Settings are saved automatically
          </p>
        </div>
      </div>
    </div>
  );
}
