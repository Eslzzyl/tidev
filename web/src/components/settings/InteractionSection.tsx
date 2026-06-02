import { useUIStore } from "../../stores/useUIStore";

export function InteractionSection() {
  const enterToSend = useUIStore((s) => s.settings.enterToSend);
  const updateSettings = useUIStore((s) => s.updateSettings);

  return (
    <section>
      <h2 className="mb-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
        Interaction
      </h2>
      <p className="mb-4 text-sm text-neutral-500 dark:text-neutral-400">
        Customize how the chat input behaves
      </p>

      <label className="flex items-center justify-between rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
        <div>
          <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            Enter to send
          </span>
          <p className="text-xs text-neutral-500">Press Enter to send, Shift+Enter for new line</p>
        </div>
        <button
          role="switch"
          aria-checked={enterToSend}
          onClick={() => updateSettings({ enterToSend: !enterToSend })}
          className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
            enterToSend
              ? "bg-neutral-900 dark:bg-neutral-100"
              : "bg-neutral-300 dark:bg-neutral-600"
          }`}
        >
          <span
            className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
              enterToSend ? "translate-x-[18px]" : "translate-x-[2px]"
            }`}
          />
        </button>
      </label>
    </section>
  );
}
