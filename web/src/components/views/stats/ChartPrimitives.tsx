import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatNumber, formatTokenBucket } from "./shared";

// ── Custom Tooltip ───────────────────────────────────────────────────────

export function ChartTooltip({
  active,
  payload,
  label,
  granularity,
}: {
  active?: boolean;
  payload?: { name: string; value: number; color: string }[];
  label?: string;
  granularity: string;
}) {
  const { t } = useTranslation();
  if (!active || !payload?.length) return null;
  return (
    <div className="rounded-lg border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
      <p className="mb-1 text-xs font-medium text-neutral-500 dark:text-neutral-400">
        {formatTokenBucket(granularity, label || "")}
      </p>
      {payload.map((p, i) => (
        <p key={i} className="flex items-center gap-2 text-xs">
          <span
            className="inline-block h-2 w-2 rounded-full"
            style={{ backgroundColor: p.color }}
          />
          <span className="text-neutral-700 dark:text-neutral-300">{p.name}:</span>
          <span className="font-medium text-neutral-900 dark:text-neutral-100">
            {p.name === t("Cache Hit Rate") ? `${p.value.toFixed(1)}%` : formatNumber(p.value)}
          </span>
        </p>
      ))}
    </div>
  );
}

// ── Chart Container (fixes zero-dimension warning) ──────────────────────
//
// ResponsiveContainer warns when it measures 0×0 on first render (which
// happens when the tab starts as display:none).  This wrapper defers
// rendering children until the container has non-zero dimensions.

export function ChartContainer({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // If already sized, show immediately
    if (el.clientWidth > 0 && el.clientHeight > 0) {
      setReady(true);
      return;
    }

    // Otherwise wait for ResizeObserver
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
          setReady(true);
          observer.disconnect();
          break;
        }
      }
    });
    observer.observe(el);

    // Fallback: check on next animation frame
    const raf = requestAnimationFrame(() => {
      if (el.clientWidth > 0) setReady(true);
    });

    return () => {
      observer.disconnect();
      cancelAnimationFrame(raf);
    };
  }, []);

  return (
    <div ref={ref} className={className}>
      {ready ? children : null}
    </div>
  );
}

export function ChartEmptyState({ text }: { text: string }) {
  return (
    <div className="flex h-[200px] items-center justify-center rounded-lg border border-dashed border-neutral-200 text-sm text-neutral-400 dark:border-neutral-800">
      {text}
    </div>
  );
}
