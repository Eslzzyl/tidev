import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import {
  LineChart,
  Line,
  AreaChart,
  Area,
  BarChart,
  Bar,
  PieChart,
  Pie,
  Cell,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import {
  BarChart3,
  Loader2,
  AlertCircle,
  Hash,
  MessagesSquare,
  Database,
  Zap,
  RefreshCw,
  MousePointerClick,
} from "lucide-react";
import { api } from "../../api/client";
import type {
  StatsSummary,
  StatsTimeSeries,
  StatsTimeSeriesEntry,
  ModelUsageEntry,
  ProviderUsageEntry,
  SessionUsageEntry,
} from "../../types/api";

// ── Color palettes ───────────────────────────────────────────────────────

const LIGHT_COLORS = [
  "#2563eb",
  "#16a34a",
  "#d97706",
  "#dc2626",
  "#7c3aed",
  "#0891b2",
  "#ca8a04",
  "#be185d",
];

const DARK_COLORS = [
  "#60a5fa",
  "#4ade80",
  "#fbbf24",
  "#f87171",
  "#a78bfa",
  "#22d3ee",
  "#fde68a",
  "#f9a8d4",
];

const CHART_COLORS = ["#2563eb", "#16a34a", "#d97706", "#dc2626", "#7c3aed"];

// ── Helper functions ─────────────────────────────────────────────────────

function formatNumber(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toLocaleString();
}

function formatTokenBucket(
  granularity: string,
  bucket: string,
): string {
  const d = new Date(bucket);
  switch (granularity) {
    case "hour":
      return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    case "day":
      return d.toLocaleDateString([], { month: "short", day: "numeric" });
    case "week":
      return `W${getWeekNumber(d)}`;
    case "month":
      return d.toLocaleDateString([], { month: "short", year: "2-digit" });
    default:
      return bucket;
  }
}

function getWeekNumber(d: Date): number {
  const startOfYear = new Date(d.getFullYear(), 0, 1);
  const diff = d.getTime() - startOfYear.getTime();
  return Math.ceil((diff / 86400000 + startOfYear.getDay() + 1) / 7);
}

function formatDate(d: string | null): string {
  if (!d) return "—";
  return new Date(d).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

type Granularity = "hour" | "day" | "week" | "month";

// ── Custom Tooltip ───────────────────────────────────────────────────────

function ChartTooltip({
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
          <span className="text-neutral-700 dark:text-neutral-300">
            {p.name}:
          </span>
          <span className="font-medium text-neutral-900 dark:text-neutral-100">
            {p.name === "Cache Hit Rate"
              ? `${p.value.toFixed(1)}%`
              : formatNumber(p.value)}
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

function ChartContainer({ children, className }: { children: React.ReactNode; className?: string }) {
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

// ── Main Component ───────────────────────────────────────────────────────

export function StatsView() {
  // Data state
  const [summary, setSummary] = useState<StatsSummary | null>(null);
  const [timeSeries, setTimeSeries] = useState<StatsTimeSeries | null>(null);
  const [models, setModels] = useState<ModelUsageEntry[]>([]);
  const [providers, setProviders] = useState<ProviderUsageEntry[]>([]);
  const [sessions, setSessions] = useState<SessionUsageEntry[]>([]);
  const [sessionTotal, setSessionTotal] = useState(0);

  // UI state
  const [granularity, setGranularity] = useState<Granularity>("hour");
  const [initialLoading, setInitialLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isDark, setIsDark] = useState(false);

  // Detect dark mode
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    setIsDark(mq.matches);
    const handler = (e: MediaQueryListEvent) => setIsDark(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  const colors = useMemo(() => (isDark ? DARK_COLORS : LIGHT_COLORS), [isDark]);

  // Use a ref so fetchAll never changes identity (avoids useEffect chain on tab switch)
  const granularityRef = useRef(granularity);
  granularityRef.current = granularity;

  // Fetch everything on mount
  const fetchAll = useCallback(async () => {
    setError(null);
    try {
      const [sum, ts, mods, provs, sessRes] = await Promise.all([
        api.getStatsSummary(),
        api.getStatsTimeSeries({ granularity: granularityRef.current }),
        api.getStatsModels(),
        api.getStatsProviders(),
        api.getStatsSessions({ limit: 10 }),
      ]);
      setSummary(sum);
      setTimeSeries(ts);
      setModels(mods.entries);
      setProviders(provs.entries);
      setSessions(sessRes.entries);
      setSessionTotal(sessRes.total);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load stats");
    } finally {
      setInitialLoading(false);
    }
    // stable identity — never recreated
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Fetch only time series when granularity changes
  const fetchTimeSeries = useCallback(async (g: Granularity) => {
    try {
      const ts = await api.getStatsTimeSeries({ granularity: g });
      setTimeSeries(ts);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load stats");
    }
  }, []);

  // Fetch on mount only
  useEffect(() => {
    fetchAll();
  }, [fetchAll]);

  // ── Derived data ─────────────────────────────────────────────────────

  const totalTokenData = useMemo(() => {
    if (!timeSeries) return [];
    return timeSeries.entries.map((e) => ({
      bucket: e.time_bucket,
      Input: e.input_tokens,
      Output: e.output_tokens,
      "Cache Read": e.cache_read_tokens,
      cacheHitRate: e.input_tokens > 0
        ? (e.cache_read_tokens / e.input_tokens) * 100
        : 0,
    }));
  }, [timeSeries]);

  const modelPieData = useMemo(() => {
    return models.slice(0, 8).map((m, i) => ({
      name: m.model_display_name || m.model_id,
      value: m.total_tokens,
      color: colors[i % colors.length],
      provider: m.provider_display_name || m.provider_id,
    }));
  }, [models, colors]);

  const totalTokens = models.reduce((s, m) => s + m.total_tokens, 0);

  // ── Loading / Error states ───────────────────────────────────────────

  if (initialLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3">
          <Loader2 className="h-6 w-6 animate-spin text-neutral-400" />
          <p className="text-sm text-neutral-500">Loading statistics…</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-center">
          <AlertCircle className="h-8 w-8 text-red-500" />
          <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
          <button
            onClick={fetchAll}
            className="rounded-lg bg-neutral-100 px-4 py-2 text-sm font-medium text-neutral-700 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  // ── Render ───────────────────────────────────────────────────────────

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-7xl space-y-6 p-4 pb-12 sm:p-6">
        {/* ── Header ──────────────────────────────────────────────────── */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <BarChart3 className="h-5 w-5 text-neutral-700 dark:text-neutral-300" />
            <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
              Statistics
            </h1>
          </div>
          <div className="flex items-center gap-3">
            {/* Granularity selector */}
            <div className="flex overflow-hidden rounded-lg border border-neutral-200 dark:border-neutral-700">
              {(["hour", "day", "week", "month"] as Granularity[]).map((g) => (
                <button
                  key={g}
                  onClick={() => {
                    setGranularity(g);
                    granularityRef.current = g;
                    fetchTimeSeries(g);
                  }}
                  className={`px-3 py-1.5 text-xs font-medium transition-colors ${
                    granularity === g
                      ? "bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900"
                      : "bg-white text-neutral-600 hover:bg-neutral-50 dark:bg-neutral-950 dark:text-neutral-400 dark:hover:bg-neutral-900"
                  }`}
                >
                  {g.charAt(0).toUpperCase() + g.slice(1)}
                </button>
              ))}
            </div>
            {/* Refresh button */}
            <button
              onClick={fetchAll}
              className="rounded-lg p-1.5 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
              title="Refresh"
            >
              <RefreshCw className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* ── Summary Cards ───────────────────────────────────────────── */}
        {summary && (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
            <SummaryCard
              icon={<Database className="h-4 w-4" />}
              label="Total Tokens"
              value={formatNumber(summary.total_tokens)}
              subtitle={`${formatNumber(summary.total_input_tokens)} in / ${formatNumber(summary.total_output_tokens)} out`}
            />
            <SummaryCard
              icon={<MousePointerClick className="h-4 w-4" />}
              label="Total Requests"
              value={summary.total_requests.toLocaleString()}
            />
            <SummaryCard
              icon={<Hash className="h-4 w-4" />}
              label="Total Sessions"
              value={formatNumber(summary.total_sessions)}
            />
            <SummaryCard
              icon={<Zap className="h-4 w-4" />}
              label="Cache Hit Rate"
              value={`${summary.cache_hit_rate.toFixed(1)}%`}
              subtitle={`${formatNumber(summary.total_cache_read_tokens)} read / ${formatNumber(summary.total_cache_write_tokens)} write`}
            />
            <SummaryCard
              icon={<MessagesSquare className="h-4 w-4" />}
              label="First Usage"
              value={formatDate(summary.first_usage_date)}
            />
          </div>
        )}

        {/* ── Token Usage ──────────────────────────────────────────────── */}
        <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
          <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
            Token Usage
          </h2>
          <ChartContainer className="h-[300px]">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={totalTokenData}>
                <defs>
                  <linearGradient id="gradInput" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={CHART_COLORS[2]} stopOpacity={0.3} />
                    <stop offset="95%" stopColor={CHART_COLORS[2]} stopOpacity={0} />
                  </linearGradient>
                  <linearGradient id="gradOutput" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={CHART_COLORS[1]} stopOpacity={0.3} />
                    <stop offset="95%" stopColor={CHART_COLORS[1]} stopOpacity={0} />
                  </linearGradient>
                  <linearGradient id="gradCache" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor={CHART_COLORS[0]} stopOpacity={0.3} />
                    <stop offset="95%" stopColor={CHART_COLORS[0]} stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid
                  strokeDasharray="3 3"
                  stroke={isDark ? "#333" : "#e5e7eb"}
                />
                <XAxis
                  dataKey="bucket"
                  tickFormatter={(v) => formatTokenBucket(granularity, v)}
                  tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                  axisLine={false}
                  tickLine={false}
                />
                <YAxis
                  yAxisId="left"
                  tickFormatter={formatNumber}
                  tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                  axisLine={false}
                  tickLine={false}
                />
                <YAxis
                  yAxisId="right"
                  orientation="right"
                  domain={[0, 100]}
                  tickFormatter={(v) => `${v}%`}
                  tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                  axisLine={false}
                  tickLine={false}
                />
                <Tooltip
                  content={<ChartTooltip granularity={granularity} />}
                />
                <Legend
                  wrapperStyle={{ fontSize: 12 }}
                  iconType="circle"
                />
                <Area
                  yAxisId="left"
                  type="monotone"
                  dataKey="Input"
                  stroke={CHART_COLORS[2]}
                  fill="url(#gradInput)"
                  strokeWidth={2}
                />
                <Area
                  yAxisId="left"
                  type="monotone"
                  dataKey="Output"
                  stroke={CHART_COLORS[1]}
                  fill="url(#gradOutput)"
                  strokeWidth={2}
                />
                <Area
                  yAxisId="left"
                  type="monotone"
                  dataKey="Cache Read"
                  stroke={CHART_COLORS[0]}
                  fill="url(#gradCache)"
                  strokeWidth={1.5}
                />
                <Line
                  yAxisId="right"
                  type="monotone"
                  dataKey="cacheHitRate"
                  stroke={CHART_COLORS[3]}
                  strokeWidth={2}
                  strokeDasharray="4 3"
                  dot={false}
                  name="Cache Hit Rate"
                />
              </AreaChart>
            </ResponsiveContainer>
          </ChartContainer>
        </div>

        {/* ── Request Count Chart (Bar) ───────────────────────────────── */}
        <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
          <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
            Requests Over Time
          </h2>
          <ChartContainer className="h-[200px]">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={timeSeries?.entries || []}>
                <CartesianGrid
                  strokeDasharray="3 3"
                  stroke={isDark ? "#333" : "#e5e7eb"}
                />
                <XAxis
                  dataKey="time_bucket"
                  tickFormatter={(v) => formatTokenBucket(granularity, v)}
                  tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                  axisLine={false}
                  tickLine={false}
                />
                <YAxis
                  tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                  axisLine={false}
                  tickLine={false}
                />
                <Tooltip
                  content={<ChartTooltip granularity={granularity} />}
                />
                <Bar
                  dataKey="request_count"
                  fill={CHART_COLORS[0]}
                  radius={[4, 4, 0, 0]}
                  maxBarSize={40}
                />
              </BarChart>
            </ResponsiveContainer>
          </ChartContainer>
        </div>

        {/* ── Model Breakdown + Provider Breakdown ──────────────────── */}
        <div className="grid gap-6 lg:grid-cols-2">
          {/* Model Pie Chart */}
          <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
            <h2 className="mb-2 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              Model Distribution (by tokens)
            </h2>
            {modelPieData.length === 0 ? (
              <p className="py-8 text-center text-sm text-neutral-400">
                No model usage data yet
              </p>
            ) : (
              <ChartContainer className="h-[280px]">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={modelPieData}
                      cx="50%"
                      cy="50%"
                      innerRadius={60}
                      outerRadius={100}
                      paddingAngle={2}
                      dataKey="value"
                    >
                      {modelPieData.map((entry, i) => (
                        <Cell key={i} fill={entry.color} />
                      ))}
                    </Pie>
                    <Tooltip
                      content={({ active, payload }) => {
                        if (!active || !payload?.length) return null;
                        const d = payload[0].payload;
                        const pct = totalTokens > 0
                          ? ((d.value / totalTokens) * 100).toFixed(1)
                          : "0";
                        return (
                          <div className="rounded-lg border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                            <p className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                              {d.name}
                            </p>
                            <p className="text-xs text-neutral-500">
                              {d.provider}
                            </p>
                            <p className="mt-1 text-xs text-neutral-700 dark:text-neutral-300">
                              {formatNumber(d.value)} tokens ({pct}%)
                            </p>
                          </div>
                        );
                      }}
                    />
                    <Legend
                      wrapperStyle={{ fontSize: 11 }}
                      iconType="circle"
                    />
                  </PieChart>
                </ResponsiveContainer>
              </ChartContainer>
            )}
          </div>

          {/* Provider Breakdown */}
          <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
            <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              Provider Usage
            </h2>
            {providers.length === 0 ? (
              <p className="py-8 text-center text-sm text-neutral-400">
                No provider data yet
              </p>
            ) : (
              <ChartContainer className="h-[280px]">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart
                    data={providers.map((p) => ({
                      ...p,
                      freshInput: Math.max(0, p.input_tokens - p.cache_read_tokens),
                      shortName: p.provider_display_name || p.provider_id,
                    }))}
                    layout="vertical"
                    stackOffset="sign"
                  >
                    <CartesianGrid
                      strokeDasharray="3 3"
                      stroke={isDark ? "#333" : "#e5e7eb"}
                      horizontal={false}
                    />
                    <XAxis
                      type="number"
                      tickFormatter={formatNumber}
                      tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                      axisLine={false}
                      tickLine={false}
                    />
                    <YAxis
                      dataKey="shortName"
                      type="category"
                      tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                      axisLine={false}
                      tickLine={false}
                      width={100}
                    />
                    <Tooltip
                      content={({ active, payload }) => {
                        if (!active || !payload?.length) return null;
                        const d = payload[0].payload as ProviderUsageEntry & { freshInput: number };
                        return (
                          <div className="rounded-lg border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                            <p className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                              {d.provider_display_name || d.provider_id}
                            </p>
                            <p className="mt-1 text-xs text-neutral-700 dark:text-neutral-300">
                              Total: {formatNumber(d.total_tokens)}
                            </p>
                            <p className="text-xs text-blue-800 dark:text-blue-300">
                              Output: {formatNumber(d.output_tokens)}
                            </p>
                            <p className="text-xs text-blue-600">
                              Fresh Input: {formatNumber(Math.max(0, d.input_tokens - d.cache_read_tokens))}
                            </p>
                            <p className="text-xs text-blue-500">
                              Cache Read: {formatNumber(d.cache_read_tokens)}
                            </p>
                            <p className="mt-1 text-xs text-neutral-500">
                              Requests: {formatNumber(d.request_count)}
                            </p>
                          </div>
                        );
                      }}
                    />
                    <Legend
                      wrapperStyle={{ fontSize: 11 }}
                      iconType="rect"
                    />
                    <Bar
                      stackId="a"
                      dataKey="output_tokens"
                      name="Output"
                      fill="#1d4ed8"
                      radius={[0, 0, 0, 0]}
                    />
                    <Bar
                      stackId="a"
                      dataKey="freshInput"
                      name="Fresh Input"
                      fill="#3b82f6"
                      radius={[0, 0, 0, 0]}
                    />
                    <Bar
                      stackId="a"
                      dataKey="cache_read_tokens"
                      name="Cache Read"
                      fill="#93c5fd"
                      radius={[0, 4, 4, 0]}
                    />
                  </BarChart>
                </ResponsiveContainer>
              </ChartContainer>
            )}
          </div>
        </div>

        {/* ── Top Sessions Table ───────────────────────────────────────── */}
        <div className="rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950">
          <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
            <h2 className="text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              Top Sessions by Token Usage
            </h2>
            <span className="text-xs text-neutral-400">
              {sessionTotal} total
            </span>
          </div>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead>
                <tr className="border-b border-neutral-100 dark:border-neutral-800">
                  <th className="px-4 py-2 font-medium text-neutral-500">Session</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Model</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Messages</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Total Tokens</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Input</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Output</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Cache Hit Rate</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">Last Active</th>
                </tr>
              </thead>
              <tbody>
                {sessions.length === 0 ? (
                  <tr>
                    <td
                      colSpan={8}
                      className="px-4 py-8 text-center text-neutral-400"
                    >
                      No session data yet
                    </td>
                  </tr>
                ) : (
                  sessions.map((s) => (
                    <tr
                      key={s.session_id}
                      className="border-b border-neutral-50 hover:bg-neutral-50 dark:border-neutral-800/50 dark:hover:bg-neutral-900/50"
                    >
                      <td className="max-w-[200px] truncate px-4 py-2.5 font-medium text-neutral-900 dark:text-neutral-100">
                        {s.title || "Untitled"}
                      </td>
                      <td className="px-4 py-2.5 text-neutral-600 dark:text-neutral-400">
                        {s.model_display_name || s.model_id}
                      </td>
                      <td className="px-4 py-2.5 text-neutral-600 dark:text-neutral-400">
                        {s.message_count}
                      </td>
                      <td className="px-4 py-2.5 font-medium text-neutral-900 dark:text-neutral-100">
                        {formatNumber(s.total_tokens)}
                      </td>
                      <td className="px-4 py-2.5 text-neutral-600 dark:text-neutral-400">
                        {formatNumber(s.input_tokens)}
                      </td>
                      <td className="px-4 py-2.5 text-neutral-600 dark:text-neutral-400">
                        {formatNumber(s.output_tokens)}
                      </td>
                      <td className="px-4 py-2.5 text-neutral-600 dark:text-neutral-400">
                        {s.input_tokens > 0
                          ? `${((s.cache_read_tokens / s.input_tokens) * 100).toFixed(1)}%`
                          : "0%"}
                      </td>
                      <td className="px-4 py-2.5 text-neutral-500">
                        {s.updated_at.split("T")[0]}
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── SummaryCard Sub-component ────────────────────────────────────────────

function SummaryCard({
  icon,
  label,
  value,
  subtitle,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  subtitle?: string;
}) {
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-4 transition-colors hover:bg-neutral-50 dark:border-neutral-800 dark:bg-neutral-950 dark:hover:bg-neutral-900">
      <div className="mb-2 flex items-center gap-2 text-neutral-500 dark:text-neutral-400">
        {icon}
        <span className="text-xs font-medium">{label}</span>
      </div>
      <div className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
        {value}
      </div>
      {subtitle && (
        <div className="mt-0.5 text-xs text-neutral-400">{subtitle}</div>
      )}
    </div>
  );
}
