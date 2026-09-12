import { useState, useEffect, useMemo } from "react";
import {
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
import { queryClient } from "../../lib/queryClient";
import { useStatsInsights, useStatsOverview, useStatsActivity } from "../../hooks/useQueries";
import type { ProviderUsageEntry } from "../../types/api";
import { useTranslation } from "react-i18next";
import { useLocation } from "wouter";
import { routes } from "../../lib/routes";
import { Button, IconButton, Tabs } from "../ui";
import { ChartContainer, ChartEmptyState, ChartTooltip } from "./stats/ChartPrimitives";
import {
  ActivityHeatmapCard,
  ActiveSessionsChart,
  ModelShareChart,
  RequestSizeDistributionChart,
  SummaryCard,
  WeeklyRhythmCard,
} from "./stats/InsightCharts";
import {
  CHART_COLORS,
  DARK_COLORS,
  LIGHT_COLORS,
  RANGE_OPTIONS,
  formatDate,
  formatNumber,
  formatRequestSizeRange,
  formatTokenBucket,
  type Granularity,
  type StatsRange,
} from "./stats/shared";

// ── Main Component ───────────────────────────────────────────────────────

export function StatsView() {
  const { t } = useTranslation();
  const [location, navigate] = useLocation();

  const getInitialParams = () => {
    if (typeof window === "undefined")
      return { range: "24h" as StatsRange, granularity: "hour" as Granularity };
    const params = new URLSearchParams(window.location.search);
    const r = params.get("range") as StatsRange | null;
    const g = params.get("granularity") as Granularity | null;
    return {
      range: r && ["24h", "7d", "30d", "all"].includes(r) ? r : ("24h" as StatsRange),
      granularity: g && ["hour", "day", "week", "month"].includes(g) ? g : ("hour" as Granularity),
    };
  };

  const initialParams = useMemo(getInitialParams, []);
  const [granularity, setGranularity] = useState<Granularity>(initialParams.granularity);
  const [range, setRange] = useState<StatsRange>(initialParams.range);
  const [isDark, setIsDark] = useState(false);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const r = params.get("range") as StatsRange | null;
    const g = params.get("granularity") as Granularity | null;
    if (r && ["24h", "7d", "30d", "all"].includes(r) && r !== range) {
      setRange(r);
    }
    if (g && ["hour", "day", "week", "month"].includes(g) && g !== granularity) {
      setGranularity(g);
    }
  }, [location]);

  const handleRangeChange = (newRange: StatsRange) => {
    setRange(newRange);
    navigate(routes.stats(newRange, granularity), { replace: true });
  };

  const handleGranularityChange = (newGranularity: Granularity) => {
    setGranularity(newGranularity);
    navigate(routes.stats(range, newGranularity), { replace: true });
  };

  // Detect dark mode
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const rafId = requestAnimationFrame(() => setIsDark(mq.matches));
    const handler = (e: MediaQueryListEvent) => setIsDark(e.matches);
    mq.addEventListener("change", handler);
    return () => {
      mq.removeEventListener("change", handler);
      cancelAnimationFrame(rafId);
    };
  }, []);

  const colors = useMemo(() => (isDark ? DARK_COLORS : LIGHT_COLORS), [isDark]);
  const statsParams = useMemo(() => {
    const option = RANGE_OPTIONS.find((candidate) => candidate.value === range);
    if (!option?.durationMs) return undefined;
    const end = new Date();
    return {
      start: new Date(end.getTime() - option.durationMs).toISOString(),
      end: end.toISOString(),
    };
  }, [range]);

  // Queries
  const overviewQ = useStatsOverview(granularity, statsParams, 10);
  const activityQ = useStatsActivity();
  const insightsQ = useStatsInsights(granularity, statsParams);

  const initialLoading = overviewQ.isLoading;
  const error = overviewQ.error;
  const errorMessage = error
    ? error instanceof Error
      ? error.message
      : t("Failed to load stats")
    : null;

  const summary = overviewQ.data?.summary;
  const timeSeries = overviewQ.data?.timeseries;
  const models = overviewQ.data?.models.entries ?? [];
  const providers = overviewQ.data?.providers.entries ?? [];
  const sessions = overviewQ.data?.sessions.entries ?? [];
  const sessionTotal = overviewQ.data?.sessions.total ?? 0;
  const insights = insightsQ.data;

  // ── Derived data ─────────────────────────────────────────────────────

  const totalTokenData = useMemo(() => {
    if (!timeSeries) return [];
    return timeSeries.entries.map((e) => ({
      bucket: e.time_bucket,
      "Fresh Input": Math.max(0, e.input_tokens - e.cache_read_tokens),
      Output: e.output_tokens,
      "Cache Read": e.cache_read_tokens,
      cacheHitRate: e.input_tokens > 0 ? (e.cache_read_tokens / e.input_tokens) * 100 : 0,
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

  const requestSizeData = useMemo(
    () =>
      (insights?.request_size_distribution ?? []).map((bucket) => ({
        ...bucket,
        label: formatRequestSizeRange(bucket.lower_bound, bucket.upper_bound),
      })),
    [insights],
  );

  // ── Loading / Error states ───────────────────────────────────────────

  if (initialLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3">
          <Loader2 className="h-6 w-6 animate-spin text-neutral-400" />
          <p className="text-sm text-neutral-500">{t("Loading statistics…")}</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-center">
          <AlertCircle className="h-8 w-8 text-red-500" />
          <p className="text-sm text-red-600 dark:text-red-400">{errorMessage}</p>
          <Button
            type="button"
            onClick={() => queryClient.invalidateQueries({ queryKey: ["stats"] })}
            variant="secondary"
            size="md"
          >
            {t("Retry")}
          </Button>
        </div>
      </div>
    );
  }

  // ── Render ───────────────────────────────────────────────────────────

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-7xl space-y-6 p-4 pb-12 sm:p-6">
        {/* ── Header ──────────────────────────────────────────────────── */}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <BarChart3 className="h-5 w-5 text-neutral-700 dark:text-neutral-300" />
            <h1 className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
              {t("Statistics")}
            </h1>
          </div>
          <div className="flex flex-wrap items-center justify-end gap-2">
            {/* Date range selector */}
            <Tabs.Root
              value={range}
              onValueChange={(value) => handleRangeChange(value as StatsRange)}
            >
              <Tabs.List className="stats-tabs-list" aria-label={t("Statistics range")}>
                {RANGE_OPTIONS.map((option) => (
                  <Tabs.Trigger
                    key={option.value}
                    value={option.value}
                    className="stats-tab-trigger"
                  >
                    {option.value === "all" ? t("All") : option.label}
                  </Tabs.Trigger>
                ))}
              </Tabs.List>
            </Tabs.Root>
            {/* Granularity selector */}
            <Tabs.Root
              value={granularity}
              onValueChange={(value) => handleGranularityChange(value as Granularity)}
            >
              <Tabs.List className="stats-tabs-list" aria-label={t("Statistics granularity")}>
                {(["hour", "day", "week", "month"] as Granularity[]).map((g) => (
                  <Tabs.Trigger key={g} value={g} className="stats-tab-trigger">
                    {t(g.charAt(0).toUpperCase() + g.slice(1))}
                  </Tabs.Trigger>
                ))}
              </Tabs.List>
            </Tabs.Root>
            {/* Refresh button */}
            <IconButton
              label={t("Refresh")}
              size="sm"
              onClick={() => queryClient.invalidateQueries({ queryKey: ["stats"] })}
              title={t("Refresh")}
            >
              <RefreshCw className="h-4 w-4" />
            </IconButton>
          </div>
        </div>

        {/* ── Summary Cards ───────────────────────────────────────────── */}
        {summary && (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
            <SummaryCard
              icon={<Database className="h-4 w-4" />}
              label={t("Total Tokens")}
              value={formatNumber(summary.total_tokens)}
              subtitle={t("{{input}} in / {{output}} out", {
                input: formatNumber(summary.total_input_tokens),
                output: formatNumber(summary.total_output_tokens),
              })}
            />
            <SummaryCard
              icon={<MousePointerClick className="h-4 w-4" />}
              label={t("Total Requests")}
              value={summary.total_requests.toLocaleString()}
            />
            <SummaryCard
              icon={<Hash className="h-4 w-4" />}
              label={t("Total Sessions")}
              value={formatNumber(summary.total_sessions)}
            />
            <SummaryCard
              icon={<Zap className="h-4 w-4" />}
              label={t("Cache Hit Rate")}
              value={`${summary.cache_hit_rate.toFixed(1)}%`}
              subtitle={t("{{read}} read / {{write}} write", {
                read: formatNumber(summary.total_cache_read_tokens),
                write: formatNumber(summary.total_cache_write_tokens),
              })}
            />
            <SummaryCard
              icon={<MessagesSquare className="h-4 w-4" />}
              label={t("First Usage")}
              value={formatDate(summary.first_usage_date)}
            />
          </div>
        )}

        <ActivityHeatmapCard
          activity={activityQ.data}
          loading={activityQ.isLoading}
          isDark={isDark}
        />

        {/* ── Token Usage ──────────────────────────────────────────────── */}
        <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
          <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
            {t("Token Usage")}
          </h2>
          {totalTokenData.length === 0 ? (
            <ChartEmptyState text={t("No token usage data yet")} />
          ) : (
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
                  <CartesianGrid strokeDasharray="3 3" stroke={isDark ? "#333" : "#e5e7eb"} />
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
                  <Tooltip content={<ChartTooltip granularity={granularity} />} />
                  <Legend wrapperStyle={{ fontSize: 12 }} iconType="circle" />
                  <Area
                    yAxisId="left"
                    type="monotone"
                    dataKey="Fresh Input"
                    name={t("Fresh Input")}
                    stroke={CHART_COLORS[2]}
                    fill="url(#gradInput)"
                    strokeWidth={2}
                  />
                  <Area
                    yAxisId="left"
                    type="monotone"
                    dataKey="Output"
                    name={t("Output")}
                    stroke={CHART_COLORS[1]}
                    fill="url(#gradOutput)"
                    strokeWidth={2}
                  />
                  <Area
                    yAxisId="left"
                    type="monotone"
                    dataKey="Cache Read"
                    name={t("Cache Read")}
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
                    name={t("Cache Hit Rate")}
                  />
                </AreaChart>
              </ResponsiveContainer>
            </ChartContainer>
          )}
        </div>

        {/* ── Request Count Chart (Bar) ───────────────────────────────── */}
        <div className="grid gap-6 lg:grid-cols-2">
          <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
            <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              {t("Requests Over Time")}
            </h2>
            {!timeSeries?.entries.length ? (
              <ChartEmptyState text={t("No request data yet")} />
            ) : (
              <ChartContainer className="h-[200px]">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart data={timeSeries.entries}>
                    <CartesianGrid strokeDasharray="3 3" stroke={isDark ? "#333" : "#e5e7eb"} />
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
                    <Tooltip content={<ChartTooltip granularity={granularity} />} />
                    <Bar
                      dataKey="request_count"
                      name={t("Requests")}
                      fill={CHART_COLORS[0]}
                      radius={[4, 4, 0, 0]}
                      maxBarSize={40}
                    />
                  </BarChart>
                </ResponsiveContainer>
              </ChartContainer>
            )}
          </div>

          <ActiveSessionsChart
            entries={insights?.active_sessions ?? []}
            granularity={insights?.granularity ?? granularity}
            loading={insightsQ.isLoading}
            isDark={isDark}
          />
        </div>

        <ModelShareChart
          insights={insights}
          granularity={insights?.granularity ?? granularity}
          loading={insightsQ.isLoading}
          isDark={isDark}
        />

        {/* ── Model Breakdown + Provider Breakdown ──────────────────── */}
        <div className="grid gap-6 lg:grid-cols-2">
          {/* Model Pie Chart */}
          <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
            <h2 className="mb-2 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              {t("Model Distribution (by tokens)")}
            </h2>
            {modelPieData.length === 0 ? (
              <p className="py-8 text-center text-sm text-neutral-400">
                {t("No model usage data yet")}
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
                        const pct =
                          totalTokens > 0 ? ((d.value / totalTokens) * 100).toFixed(1) : "0";
                        return (
                          <div className="rounded-lg border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                            <p className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                              {d.name}
                            </p>
                            <p className="text-xs text-neutral-500">{d.provider}</p>
                            <p className="mt-1 text-xs text-neutral-700 dark:text-neutral-300">
                              {t("{{value}} tokens ({{percent}}%)", {
                                value: formatNumber(d.value),
                                percent: pct,
                              })}
                            </p>
                          </div>
                        );
                      }}
                    />
                    <Legend wrapperStyle={{ fontSize: 11 }} iconType="circle" />
                  </PieChart>
                </ResponsiveContainer>
              </ChartContainer>
            )}
          </div>

          {/* Provider Breakdown */}
          <div className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
            <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              {t("Provider Usage")}
            </h2>
            {providers.length === 0 ? (
              <p className="py-8 text-center text-sm text-neutral-400">
                {t("No provider data yet")}
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
                        const d = payload[0].payload as ProviderUsageEntry & {
                          freshInput: number;
                        };
                        return (
                          <div className="rounded-lg border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                            <p className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                              {d.provider_display_name || d.provider_id}
                            </p>
                            <p className="mt-1 text-xs text-neutral-700 dark:text-neutral-300">
                              {t("Total: {{value}}", { value: formatNumber(d.total_tokens) })}
                            </p>
                            <p className="text-xs text-blue-800 dark:text-blue-300">
                              {t("Output: {{value}}", { value: formatNumber(d.output_tokens) })}
                            </p>
                            <p className="text-xs text-blue-600">
                              {t("Fresh Input: {{value}}", {
                                value: formatNumber(
                                  Math.max(0, d.input_tokens - d.cache_read_tokens),
                                ),
                              })}
                            </p>
                            <p className="text-xs text-blue-500">
                              {t("Cache Read: {{value}}", {
                                value: formatNumber(d.cache_read_tokens),
                              })}
                            </p>
                            <p className="mt-1 text-xs text-neutral-500">
                              {t("Requests: {{value}}", {
                                value: formatNumber(d.request_count),
                              })}
                            </p>
                          </div>
                        );
                      }}
                    />
                    <Legend wrapperStyle={{ fontSize: 11 }} iconType="rect" />
                    <Bar
                      stackId="a"
                      dataKey="output_tokens"
                      name={t("Output")}
                      fill="#1d4ed8"
                      radius={[0, 0, 0, 0]}
                    />
                    <Bar
                      stackId="a"
                      dataKey="freshInput"
                      name={t("Fresh Input")}
                      fill="#3b82f6"
                      radius={[0, 0, 0, 0]}
                    />
                    <Bar
                      stackId="a"
                      dataKey="cache_read_tokens"
                      name={t("Cache Read")}
                      fill="#93c5fd"
                      radius={[0, 4, 4, 0]}
                    />
                  </BarChart>
                </ResponsiveContainer>
              </ChartContainer>
            )}
          </div>
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <WeeklyRhythmCard insights={insights} loading={insightsQ.isLoading} isDark={isDark} />
          <RequestSizeDistributionChart
            data={requestSizeData}
            loading={insightsQ.isLoading}
            isDark={isDark}
          />
        </div>

        {/* ── Top Sessions Table ───────────────────────────────────────── */}
        <div className="rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950">
          <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
            <h2 className="text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              {t("Top Sessions by Token Usage")}
            </h2>
            <span className="text-xs text-neutral-400">
              {t("{{count}} total", { count: sessionTotal })}
            </span>
          </div>
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead>
                <tr className="border-b border-neutral-100 dark:border-neutral-800">
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Session")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Model")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Messages")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Total Tokens")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Input")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Output")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Cache Hit Rate")}</th>
                  <th className="px-4 py-2 font-medium text-neutral-500">{t("Last Active")}</th>
                </tr>
              </thead>
              <tbody>
                {sessions.length === 0 ? (
                  <tr>
                    <td colSpan={8} className="px-4 py-8 text-center text-neutral-400">
                      {t("No session data yet")}
                    </td>
                  </tr>
                ) : (
                  sessions.map((s) => (
                    <tr
                      key={s.session_id}
                      className="border-b border-neutral-50 hover:bg-neutral-50 dark:border-neutral-800/50 dark:hover:bg-neutral-900/50"
                    >
                      <td className="max-w-[200px] truncate px-4 py-2.5 font-medium text-neutral-900 dark:text-neutral-100">
                        {s.title || t("Untitled")}
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
                      <td className="px-4 py-2.5 text-neutral-500">{formatDate(s.updated_at)}</td>
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
