import { Area, AreaChart, Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type { UsageDay, UsageRange } from "../../../types";
import { compactTokens, dayAxisLabel, dayLabel } from "./format";

const tick = { className: "fill-fg-subtle font-mono text-caption uppercase" };

function ChartTooltip({ active, payload }: { active?: boolean; payload?: ReadonlyArray<{ payload?: UsageDay }> }) {
  const day = payload?.[0]?.payload;
  if (!active || !day) return null;
  return (
    <div className="rounded-lg border border-line-strong bg-surface-muted px-3 py-2 text-caption shadow-pop">
      <p className="font-medium text-fg">{dayLabel(day.day)}</p>
      <p className="text-fg-muted tabular-nums">
        {compactTokens(day.tokens)} tokens · {day.runs.toLocaleString()} {day.runs === 1 ? "run" : "runs"}
      </p>
      {day.cacheReadTokens > 0 && (
        <p className="text-fg-subtle tabular-nums">+ {compactTokens(day.cacheReadTokens)} read from cache</p>
      )}
    </div>
  );
}

/** Tokens per local day: bars for a week, an area for longer ranges. */
export function UsageChart({ days, byDay }: { days: UsageRange; byDay: UsageDay[] }) {
  const peak = byDay.reduce<UsageDay | null>((best, day) => (!best || day.tokens > best.tokens ? day : best), null);
  const axes = [
    <CartesianGrid key="grid" vertical={false} stroke="var(--color-line)" />,
    <XAxis
      key="x"
      dataKey="day"
      tickFormatter={dayAxisLabel}
      tick={tick}
      tickLine={false}
      axisLine={{ stroke: "var(--color-line)" }}
      interval="preserveStartEnd"
      minTickGap={24}
    />,
    <YAxis
      key="y"
      tickFormatter={compactTokens}
      tick={tick}
      tickLine={false}
      axisLine={false}
      width={48}
      tickCount={4}
      allowDecimals={false}
    />,
    <Tooltip
      key="tooltip"
      content={ChartTooltip}
      cursor={days === 7 ? { fill: "var(--color-line)" } : { stroke: "var(--color-line-strong)" }}
      isAnimationActive={false}
    />,
  ];
  return (
    <figure
      className="flex min-w-0 flex-col gap-2"
      aria-label={
        peak ? `Tokens per day, peaking at ${compactTokens(peak.tokens)} on ${dayLabel(peak.day)}` : "Tokens per day"
      }
    >
      <figcaption className="text-caption text-fg-subtle">tokens/day</figcaption>
      <div className="h-48 w-full min-w-0">
        <ResponsiveContainer width="100%" height="100%">
          {days === 7 ? (
            <BarChart data={byDay} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
              {axes}
              <Bar dataKey="tokens" fill="var(--color-brand)" radius={[4, 4, 0, 0]} maxBarSize={40} />
            </BarChart>
          ) : (
            <AreaChart data={byDay} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
              {axes}
              <Area
                dataKey="tokens"
                type="monotone"
                stroke="var(--color-brand)"
                strokeWidth={2}
                fill="var(--color-brand)"
                fillOpacity={0.14}
                activeDot={{ r: 3, fill: "var(--color-brand)", stroke: "var(--color-canvas)" }}
              />
            </AreaChart>
          )}
        </ResponsiveContainer>
      </div>
    </figure>
  );
}
