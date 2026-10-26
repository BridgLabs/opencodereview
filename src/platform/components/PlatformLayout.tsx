import { useQuery } from "@tanstack/react-query";
import { Link, Outlet, useLocation } from "@tanstack/react-router";
import { Database, FileText, GitPullRequest, type LucideIcon, SettingsIcon } from "lucide-react";
import type { ReactNode } from "react";
import { BrandMark } from "../../components/BrandMark";
import { setupApi } from "../../services/setupApi";
import { reviewNeedsAttention } from "../lib/reviewFlow";
import { EnginePicker } from "./EnginePicker";

const railItem = "group relative grid size-10 flex-none place-items-center rounded-xl no-underline transition-colors";
const railActive = "bg-surface-muted text-fg";
const railIdle = "text-fg-subtle hover:bg-surface/80 hover:text-fg";
const navigation = [
  { to: "/platform/reviews", label: "Reviews", icon: GitPullRequest },
  { to: "/platform/indexing", label: "Indexing", icon: Database },
  { to: "/platform/skills", label: "Skills", icon: FileText },
] as const;
const pageTitles: Record<string, string> = {
  "/platform/reviews": "Reviews",
  "/platform/indexing": "Indexing",
  "/platform/skills": "Skills",
  "/platform/settings": "Settings",
};

function RailTooltip({ label }: { label: string }) {
  return (
    <span
      role="tooltip"
      className="pointer-events-none absolute top-1/2 left-full z-50 ml-2.5 -translate-y-1/2 rounded-lg border border-line-strong bg-surface-muted px-2.5 py-1 text-caption font-medium whitespace-nowrap text-fg opacity-0 shadow-pop transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
    >
      {label}
    </span>
  );
}

function RailLink({
  to,
  label,
  icon: Icon,
  badge,
}: {
  to: string;
  label: string;
  icon: LucideIcon;
  badge?: ReactNode;
}) {
  return (
    <Link
      to={to}
      aria-label={label}
      className={railItem}
      activeProps={{ className: railActive }}
      inactiveProps={{ className: railIdle }}
    >
      <Icon className="size-[18px]" strokeWidth={1.75} />
      {badge}
      <RailTooltip label={label} />
    </Link>
  );
}

export function PlatformLayout() {
  const pathname = useLocation({ select: (location) => location.pathname });
  const reviewWorkspace = [
    "/platform/reviews",
    "/platform/indexing",
    "/platform/skills",
    "/platform/settings",
  ].includes(pathname);
  const pageTitle = pageTitles[pathname] ?? "";
  const reviewBoard = useQuery({
    queryKey: ["reviews", "board"],
    queryFn: setupApi.listReviewBoard,
    refetchInterval: 5000,
  });
  const reviewNeedsYou = (reviewBoard.data ?? []).flatMap((group) => group.items).filter(reviewNeedsAttention).length;
  const badge = (count: number, title: string) =>
    count > 0 && (
      <span
        className="absolute -top-0.5 -right-0.5 inline-grid h-4 min-w-4 place-items-center rounded-full bg-amber-500 px-1 text-[0.6429rem] leading-none font-semibold text-black tabular-nums ring-2 ring-rail"
        title={title}
      >
        {count}
      </span>
    );
  return (
    <div className="flex h-dvh min-w-0 flex-col overflow-hidden bg-rail text-fg">
      <header data-topbar className="flex h-11 flex-none items-center gap-3 pr-2 pl-3">
        <Link
          to="/platform/reviews"
          className="inline-flex h-8 flex-none items-center gap-2 rounded-lg px-1.5 text-small font-semibold text-fg no-underline transition-colors hover:bg-surface/80 [&_.brand-mark]:size-6!"
          aria-label="OpenCodeReview home"
        >
          <BrandMark size="sm" />
          <span>OpenCodeReview</span>
        </Link>
        {pageTitle && <span className="truncate text-small font-medium text-fg-muted max-sm:hidden">{pageTitle}</span>}
        <EnginePicker />
      </header>
      <div className="flex min-h-0 flex-1">
        <nav className="flex w-15 flex-none flex-col items-center gap-1.5 pt-1 pb-3" aria-label="Platform navigation">
          {navigation.map(({ to, label, icon }) => (
            <RailLink
              key={to}
              to={to}
              label={label}
              icon={icon}
              badge={to === "/platform/reviews" ? badge(reviewNeedsYou, "Reviews needing your attention") : undefined}
            />
          ))}
          <div className="mt-auto flex flex-col items-center gap-1.5">
            <RailLink to="/platform/settings" label="Settings" icon={SettingsIcon} />
          </div>
        </nav>
        <main
          data-shell={reviewWorkspace ? "workspace" : "page"}
          className={`min-h-0 min-w-0 flex-1 rounded-tl-xl border-t border-l border-line bg-canvas ${reviewWorkspace ? "flex flex-col overflow-hidden" : "overflow-y-auto pb-8 [scrollbar-gutter:stable]"}`}
        >
          <Outlet />
        </main>
      </div>
    </div>
  );
}
