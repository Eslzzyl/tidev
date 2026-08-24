export interface PerformanceSpan {
  name: string;
  startMark: string;
  details?: Record<string, boolean | number | string>;
}

let spanSequence = 0;
let browserObserversInstalled = false;
const INTERACTIVE_EVENT_NAMES = new Set(["click", "keydown", "pointerdown", "wheel"]);

interface ProfilerBucket {
  name: string;
  phase: string;
  actualDuration: number;
  baseDuration: number;
  renderStartTime: number;
  commitTime: number;
  componentCount: number;
  details: Record<string, boolean | number | string>;
}

const profilerBuckets = new Map<string, ProfilerBucket>();
let profilerFlushScheduled = false;

export function beginPerformance(
  name: string,
  details?: Record<string, boolean | number | string>,
): PerformanceSpan | null {
  if (typeof window === "undefined" || typeof window.performance === "undefined") return null;
  const startMark = `tidev-perf:${name}:start:${spanSequence++}`;
  window.performance.mark(startMark);
  return { name, startMark, details };
}

export function endPerformance(
  span: PerformanceSpan | null,
  details?: Record<string, boolean | number | string>,
) {
  if (!span || typeof window === "undefined" || typeof window.performance === "undefined") return;
  const endMark = `tidev-perf:${span.name}:end:${spanSequence++}`;
  try {
    window.performance.mark(endMark);
    const measure = window.performance.measure(span.name, span.startMark, endMark);
    console.info("[tidev perf]", {
      name: span.name,
      durationMs: Number(measure.duration.toFixed(1)),
      startTimeMs: Number(measure.startTime.toFixed(1)),
      endTimeMs: Number((measure.startTime + measure.duration).toFixed(1)),
      ...span.details,
      ...details,
    });
  } catch (error) {
    console.warn("[tidev perf] failed to measure", span.name, error);
  }
}

export function recordPerformance(
  name: string,
  durationMs: number,
  details?: Record<string, boolean | number | string>,
) {
  if (typeof window === "undefined") return;
  console.info("[tidev perf]", {
    name,
    durationMs: Number(durationMs.toFixed(1)),
    ...details,
  });
}

export function installPerformanceObservers() {
  if (
    browserObserversInstalled ||
    typeof window === "undefined" ||
    typeof PerformanceObserver === "undefined"
  ) {
    return;
  }
  browserObserversInstalled = true;

  function observe(type: string, callback: (entry: PerformanceEntry) => void) {
    try {
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) callback(entry);
      });
      observer.observe({ type, buffered: true });
    } catch {
      // Entry types are browser-dependent.
    }
  }

  observe("longtask", (entry) => {
    recordPerformance("browser.long-task", entry.duration, {
      startTimeMs: Number(entry.startTime.toFixed(1)),
      attributionCount: Array.isArray(
        (entry as PerformanceEntry & { attribution?: unknown[] }).attribution,
      )
        ? ((entry as PerformanceEntry & { attribution?: unknown[] }).attribution?.length ?? 0)
        : 0,
    });
  });

  observe("long-animation-frame", (entry) => {
    if (entry.duration < 50) return;
    const frame = entry as PerformanceEntry & {
      blockingDuration?: number;
      renderStart?: number;
      styleAndLayoutStart?: number;
    };
    recordPerformance("browser.long-animation-frame", entry.duration, {
      startTimeMs: Number(entry.startTime.toFixed(1)),
      blockingDurationMs:
        typeof frame.blockingDuration === "number"
          ? Number(frame.blockingDuration.toFixed(1))
          : 0,
      renderStartMs:
        typeof frame.renderStart === "number" ? Number(frame.renderStart.toFixed(1)) : 0,
      styleAndLayoutStartMs:
        typeof frame.styleAndLayoutStart === "number"
          ? Number(frame.styleAndLayoutStart.toFixed(1))
          : 0,
    });
  });

  observe("event", (entry) => {
    if (entry.duration < 50) return;
    const event = entry as PerformanceEntry & {
      processingStart?: number;
      processingEnd?: number;
    };
    if (!INTERACTIVE_EVENT_NAMES.has(entry.name)) return;
    recordPerformance("browser.event-delay", entry.duration, {
      event: entry.name,
      inputDelayMs:
        typeof event.processingStart === "number"
          ? Number((event.processingStart - entry.startTime).toFixed(1))
          : 0,
      processingDurationMs:
        typeof event.processingStart === "number" && typeof event.processingEnd === "number"
          ? Number((event.processingEnd - event.processingStart).toFixed(1))
          : 0,
    });
  });
}

export function recordReactProfiler(
  name: string,
  phase: string,
  actualDuration: number,
  baseDuration: number,
  startTime: number,
  commitTime: number,
  details: Record<string, boolean | number | string> = {},
) {
  if (typeof window === "undefined") return;
  const group = typeof details.group === "string" ? details.group : "";
  const key = `${name}:${commitTime}:${group}`;
  const bucket = profilerBuckets.get(key);
  if (bucket) {
    bucket.actualDuration += actualDuration;
    bucket.baseDuration += baseDuration;
    bucket.renderStartTime = Math.min(bucket.renderStartTime, startTime);
    bucket.componentCount += 1;
    Object.assign(bucket.details, details);
  } else {
    profilerBuckets.set(key, {
      name,
      phase,
      actualDuration,
      baseDuration,
      renderStartTime: startTime,
      commitTime,
      componentCount: 1,
      details: { ...details },
    });
  }

  if (profilerFlushScheduled) return;
  profilerFlushScheduled = true;
  const flush = () => {
    profilerFlushScheduled = false;
    const buckets = [...profilerBuckets.values()];
    profilerBuckets.clear();
    for (const item of buckets) {
      recordPerformance(item.name, item.actualDuration, {
        phase: item.phase,
        baseDurationMs: item.baseDuration,
        renderToCommitMs: item.commitTime - item.renderStartTime,
        componentCount: item.componentCount,
        commitTimeMs: item.commitTime,
        ...item.details,
      });
    }
  };
  if (typeof window.queueMicrotask === "function") {
    window.queueMicrotask(flush);
  } else {
    void Promise.resolve().then(flush);
  }
}

export function schedulePerformanceFrame(callback: () => void) {
  if (typeof window === "undefined") return () => undefined;
  if (typeof window.requestAnimationFrame === "function") {
    const frame = window.requestAnimationFrame(callback);
    return () => window.cancelAnimationFrame(frame);
  }
  const timeout = window.setTimeout(callback, 0);
  return () => window.clearTimeout(timeout);
}
