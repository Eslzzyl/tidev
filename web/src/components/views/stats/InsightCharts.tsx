import { useMemo } from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { CalendarDays } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { StatsActivity, StatsInsights } from "../../../types/api";
import i18n from "../../../i18n";
import { formatNumber, formatTokenBucket } from "./shared";
import { ChartContainer, ChartEmptyState, ChartTooltip } from "./ChartPrimitives";

// ── SummaryCard Sub-component ────────────────────────────────────────────

export function SummaryCard({
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
      <div className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">{value}</div>
      {subtitle && <div className="mt-0.5 text-xs text-neutral-400">{subtitle}</div>}
    </div>
  );
}

export function ActiveSessionsChart({
  entries,
  granularity,
  loading,
  isDark,
}: {
  entries: StatsInsights["active_sessions"];
  granularity: string;
  loading: boolean;
  isDark: boolean;
}) {
  const { t } = useTranslation();
  return (
    <section className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
      <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
        {t("Active Sessions Over Time")}
      </h2>
      {loading ? (
        <div className="h-[200px] animate-pulse rounded-lg bg-neutral-100 dark:bg-neutral-900" />
      ) : entries.length === 0 ? (
        <ChartEmptyState text={t("No active session data yet")} />
      ) : (
        <ChartContainer className="h-[200px]">
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={entries}>
              <defs>
                <linearGradient id="gradActiveSessions" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#2563eb" stopOpacity={0.28} />
                  <stop offset="95%" stopColor="#2563eb" stopOpacity={0} />
                </linearGradient>
              </defs>
              <CartesianGrid strokeDasharray="3 3" stroke={isDark ? "#333" : "#e5e7eb"} />
              <XAxis
                dataKey="time_bucket"
                tickFormatter={(value) => formatTokenBucket(granularity, value)}
                tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                axisLine={false}
                tickLine={false}
              />
              <YAxis
                allowDecimals={false}
                tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                axisLine={false}
                tickLine={false}
              />
              <Tooltip content={<ChartTooltip granularity={granularity} />} />
              <Area
                type="monotone"
                dataKey="active_sessions"
                name={t("Active Sessions")}
                stroke="#2563eb"
                fill="url(#gradActiveSessions)"
                strokeWidth={2}
                isAnimationActive={false}
              />
            </AreaChart>
          </ResponsiveContainer>
        </ChartContainer>
      )}
    </section>
  );
}

const MODEL_MIX_COLORS = ["#1e3a8a", "#1d4ed8", "#2563eb", "#3b82f6", "#60a5fa", "#93c5fd"];

export function ModelShareChart({
  insights,
  granularity,
  loading,
  isDark,
}: {
  insights?: StatsInsights;
  granularity: string;
  loading: boolean;
  isDark: boolean;
}) {
  const { t } = useTranslation();
  const modelMix = insights?.model_mix;
  return (
    <section className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
      <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
        {t("Model Share Over Time")}
      </h2>
      {loading ? (
        <div className="h-[260px] animate-pulse rounded-lg bg-neutral-100 dark:bg-neutral-900" />
      ) : !modelMix?.points.length ? (
        <ChartEmptyState text={t("No model share data yet")} />
      ) : (
        <ChartContainer className="h-[260px]">
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={modelMix.points}>
              <CartesianGrid strokeDasharray="3 3" stroke={isDark ? "#333" : "#e5e7eb"} />
              <XAxis
                dataKey="time_bucket"
                tickFormatter={(value) => formatTokenBucket(granularity, value)}
                tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                axisLine={false}
                tickLine={false}
              />
              <YAxis
                domain={[0, 100]}
                tickFormatter={(value) => `${value}%`}
                tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                axisLine={false}
                tickLine={false}
              />
              <Tooltip
                labelFormatter={(value) => formatTokenBucket(granularity, String(value ?? ""))}
                formatter={(value, name) => [
                  `${(typeof value === "number" ? value : Number(value ?? 0)).toFixed(1)}%`,
                  String(name ?? ""),
                ]}
                contentStyle={{
                  borderColor: isDark ? "#404040" : "#e5e7eb",
                  backgroundColor: isDark ? "#171717" : "#fff",
                  borderRadius: 8,
                  fontSize: 12,
                }}
              />
              <Legend wrapperStyle={{ fontSize: 11 }} iconType="circle" />
              {modelMix.series.map((series, index) => {
                const name = series.is_other
                  ? t("Other")
                  : [series.provider_display_name, series.model_display_name]
                      .filter(Boolean)
                      .join(" · ");
                return (
                  <Area
                    key={series.key}
                    type="monotone"
                    dataKey={`shares.${series.key}`}
                    name={name}
                    stackId="model-share"
                    stroke={MODEL_MIX_COLORS[index % MODEL_MIX_COLORS.length]}
                    fill={MODEL_MIX_COLORS[index % MODEL_MIX_COLORS.length]}
                    fillOpacity={0.72}
                    isAnimationActive={false}
                  />
                );
              })}
            </AreaChart>
          </ResponsiveContainer>
        </ChartContainer>
      )}
    </section>
  );
}

export function WeeklyRhythmCard({
  insights,
  loading,
  isDark,
}: {
  insights?: StatsInsights;
  loading: boolean;
  isDark: boolean;
}) {
  const { t } = useTranslation();
  const cellsByTime = useMemo(
    () =>
      new Map((insights?.rhythm.cells ?? []).map((cell) => [`${cell.weekday}-${cell.hour}`, cell])),
    [insights],
  );
  const colors = isDark ? ACTIVITY_DARK_COLORS : ACTIVITY_LIGHT_COLORS;

  return (
    <section className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
      <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
        {t("Weekly Rhythm")}
      </h2>
      {loading ? (
        <div className="h-[200px] animate-pulse rounded-lg bg-neutral-100 dark:bg-neutral-900" />
      ) : !insights?.rhythm.cells.some((cell) => cell.request_count > 0) ? (
        <ChartEmptyState text={t("No activity rhythm data yet")} />
      ) : (
        <div className="overflow-hidden pb-1">
          <div className="mx-auto flex w-full min-w-0">
            <div className="mr-3 mt-[17px] grid w-6 shrink-0 self-stretch grid-rows-7 gap-[3px] text-right text-[10px] text-neutral-500 dark:text-neutral-400">
              <span className="flex items-center justify-end">{t("Mon")}</span>
              <span />
              <span className="flex items-center justify-end">{t("Wed")}</span>
              <span />
              <span className="flex items-center justify-end">{t("Fri")}</span>
              <span />
              <span />
            </div>
            <div className="min-w-0 flex-1">
              <div
                className="mb-[3px] grid h-[14px] w-full gap-[3px] overflow-hidden text-[10px] leading-[14px] text-neutral-500 dark:text-neutral-400"
                style={{ gridTemplateColumns: "repeat(24, minmax(0, 1fr))" }}
              >
                {Array.from({ length: 24 }, (_, hour) => (
                  <span key={hour} className={hour % 4 === 0 ? "text-left" : "invisible"}>
                    {hour}
                  </span>
                ))}
              </div>
              <div
                aria-label={t("Weekly Rhythm")}
                className="grid w-full gap-[3px]"
                role="grid"
                style={{ gridTemplateColumns: "repeat(24, minmax(0, 1fr))" }}
              >
                {Array.from({ length: 7 }, (_, weekday) =>
                  Array.from({ length: 24 }, (_, hour) => {
                    const cell = cellsByTime.get(`${weekday}-${hour}`);
                    const requestCount = cell?.request_count ?? 0;
                    const totalTokens = cell?.total_tokens ?? 0;
                    return (
                      <span
                        key={`${weekday}-${hour}`}
                        aria-label={`${t("Hour")} ${hour}: ${requestCount.toLocaleString(i18n.language)} ${t("Requests")} · ${formatNumber(totalTokens)} ${t("Total Tokens")}`}
                        className="aspect-square w-full rounded-[4px] ring-1 ring-inset ring-black/[0.045] transition duration-150 hover:z-10 hover:scale-110 hover:ring-2 hover:ring-blue-500/50 dark:ring-white/[0.06] dark:hover:ring-blue-300/50"
                        role="gridcell"
                        style={{ backgroundColor: colors[cell?.level ?? 0] ?? colors[0] }}
                        title={`${t("Hour")} ${hour}: ${requestCount.toLocaleString(i18n.language)} ${t("Requests")} · ${formatNumber(totalTokens)} ${t("Total Tokens")}`}
                      />
                    );
                  }),
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

export function RequestSizeDistributionChart({
  data,
  loading,
  isDark,
}: {
  data: Array<{
    label: string;
    lower_bound: number;
    upper_bound: number | null;
    request_count: number;
    total_tokens: number;
  }>;
  loading: boolean;
  isDark: boolean;
}) {
  const { t } = useTranslation();
  return (
    <section className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
      <h2 className="mb-4 text-sm font-semibold text-neutral-700 dark:text-neutral-300">
        {t("Request Size Distribution")}
      </h2>
      {loading ? (
        <div className="h-[200px] animate-pulse rounded-lg bg-neutral-100 dark:bg-neutral-900" />
      ) : data.every((bucket) => bucket.request_count === 0) ? (
        <ChartEmptyState text={t("No request size data yet")} />
      ) : (
        <ChartContainer className="h-[200px]">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={data}>
              <CartesianGrid strokeDasharray="3 3" stroke={isDark ? "#333" : "#e5e7eb"} />
              <XAxis
                dataKey="label"
                tick={{ fontSize: 10, fill: isDark ? "#888" : "#6b7280" }}
                axisLine={false}
                tickLine={false}
              />
              <YAxis
                allowDecimals={false}
                tick={{ fontSize: 11, fill: isDark ? "#888" : "#6b7280" }}
                axisLine={false}
                tickLine={false}
              />
              <Tooltip
                content={({ active, payload }) => {
                  if (!active || !payload?.length) return null;
                  const bucket = payload[0].payload as (typeof data)[number];
                  return (
                    <div className="rounded-lg border border-neutral-200 bg-white p-3 shadow-lg dark:border-neutral-700 dark:bg-neutral-900">
                      <p className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
                        {bucket.label} {t("Total Tokens")}
                      </p>
                      <p className="mt-1 text-xs text-neutral-700 dark:text-neutral-300">
                        {t("Requests: {{value}}", {
                          value: formatNumber(bucket.request_count),
                        })}
                      </p>
                      <p className="text-xs text-neutral-500">
                        {t("Total: {{value}}", { value: formatNumber(bucket.total_tokens) })}
                      </p>
                    </div>
                  );
                }}
              />
              <Bar
                dataKey="request_count"
                name={t("Requests")}
                fill="#2563eb"
                radius={[4, 4, 0, 0]}
                maxBarSize={48}
                isAnimationActive={false}
              />
            </BarChart>
          </ResponsiveContainer>
        </ChartContainer>
      )}
    </section>
  );
}

const ACTIVITY_LIGHT_COLORS = ["#ebedf0", "#bfdbfe", "#60a5fa", "#2563eb", "#1e3a8a"];
const ACTIVITY_DARK_COLORS = ["#161b22", "#0c2d6b", "#0b4aa2", "#1f6feb", "#58a6ff"];

export function ActivityHeatmapCard({
  activity,
  loading,
  isDark,
}: {
  activity?: StatsActivity;
  loading: boolean;
  isDark: boolean;
}) {
  const { t } = useTranslation();
  const heatmap = useMemo(() => {
    const cells = activity?.cells ?? [];
    const firstDate = cells[0]?.date;
    const leading = firstDate ? (new Date(`${firstDate}T00:00:00Z`).getUTCDay() + 6) % 7 : 0;
    const monthLabels: Array<{ column: number; label: string }> = [];
    let previousMonth = -1;
    for (const [index, cell] of cells.entries()) {
      const date = new Date(`${cell.date}T00:00:00Z`);
      const month = date.getUTCMonth();
      if (month !== previousMonth) {
        monthLabels.push({
          column: Math.floor((leading + index) / 7) + 1,
          label: date.toLocaleDateString(i18n.language, { month: "short", timeZone: "UTC" }),
        });
        previousMonth = month;
      }
    }
    return {
      cells: [...Array.from({ length: leading }, () => null), ...cells],
      columns: Math.max(1, Math.ceil((leading + cells.length) / 7)),
      monthLabels,
    };
  }, [activity]);
  const colors = isDark ? ACTIVITY_DARK_COLORS : ACTIVITY_LIGHT_COLORS;

  return (
    <section className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-950">
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <CalendarDays className="h-4 w-4 text-neutral-500 dark:text-neutral-400" />
            <h2 className="text-sm font-semibold text-neutral-700 dark:text-neutral-300">
              {t("Model Activity")}
            </h2>
          </div>
          <p className="mt-1 text-sm text-neutral-500 dark:text-neutral-400">
            {loading
              ? t("Loading activity…")
              : t("{{count}} model responses in the last year", {
                  count: activity?.total_requests.toLocaleString(i18n.language) ?? "0",
                })}
          </p>
        </div>
        <div className="flex items-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
          <span>{t("Less")}</span>
          {colors.map((color, index) => (
            <span
              key={index}
              className="h-3 w-3 rounded-[3px] ring-1 ring-inset ring-black/[0.045] dark:ring-white/[0.06]"
              style={{ backgroundColor: color }}
            />
          ))}
          <span>{t("More")}</span>
        </div>
      </div>

      {loading ? (
        <div className="h-32 animate-pulse rounded-lg bg-neutral-100 dark:bg-neutral-900" />
      ) : (
        <div className="overflow-hidden pb-1">
          <div className="mx-auto flex w-full min-w-0">
            <div className="mr-3 mt-[17px] grid w-6 shrink-0 self-stretch grid-rows-7 gap-[3px] text-right text-[10px] text-neutral-500 dark:text-neutral-400">
              <span className="flex items-center justify-end">{t("Mon")}</span>
              <span />
              <span className="flex items-center justify-end">{t("Wed")}</span>
              <span />
              <span className="flex items-center justify-end">{t("Fri")}</span>
              <span />
              <span />
            </div>
            <div className="min-w-0 flex-1">
              <div
                className="mb-[3px] grid h-[14px] w-full gap-[3px] overflow-hidden text-[11px] leading-[14px] text-neutral-500 dark:text-neutral-400"
                style={{ gridTemplateColumns: `repeat(${heatmap.columns}, minmax(0, 1fr))` }}
              >
                {heatmap.monthLabels.map(({ column, label }) => (
                  <span
                    key={`${column}-${label}`}
                    className="-translate-y-px overflow-hidden whitespace-nowrap"
                    style={{ gridColumnStart: column }}
                  >
                    {label}
                  </span>
                ))}
              </div>
              <div
                aria-label={t("Model activity in the last year")}
                className="grid w-full grid-flow-col gap-[3px]"
                role="grid"
                style={{
                  gridTemplateColumns: `repeat(${heatmap.columns}, minmax(0, 1fr))`,
                  gridTemplateRows: "repeat(7, auto)",
                }}
              >
                {heatmap.cells.map((cell, index) =>
                  cell ? (
                    <span
                      key={cell.date}
                      aria-label={t("{{date}}: {{requests}} responses, {{tokens}} tokens", {
                        date: new Date(`${cell.date}T00:00:00Z`).toLocaleDateString(i18n.language, {
                          year: "numeric",
                          month: "short",
                          day: "numeric",
                          timeZone: "UTC",
                        }),
                        requests: cell.request_count.toLocaleString(i18n.language),
                        tokens: formatNumber(cell.total_tokens),
                      })}
                      className="relative aspect-square w-full rounded-[4px] ring-1 ring-inset ring-black/[0.045] transition duration-150 hover:z-10 hover:scale-110 hover:ring-2 hover:ring-blue-500/50 dark:ring-white/[0.06] dark:hover:ring-blue-300/50"
                      role="gridcell"
                      style={{ backgroundColor: colors[cell.level] ?? colors[0] }}
                      title={`${cell.date}: ${cell.request_count.toLocaleString(i18n.language)} ${t("Requests")} · ${formatNumber(cell.total_tokens)} ${t("Total Tokens")}`}
                    />
                  ) : (
                    <span
                      key={`leading-${index}`}
                      aria-hidden="true"
                      className="aspect-square w-full"
                    />
                  ),
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
