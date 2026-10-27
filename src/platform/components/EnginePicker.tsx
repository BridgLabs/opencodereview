import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Check, ChevronDown, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cx } from "../../components/DesignSystem";
import { setupApi } from "../../services/setupApi";
import type { AiAccount } from "../../types";
import { ACCOUNTS_QUERY_KEY, nextAccountId, ProviderGlyph, providerName } from "./AccountsPanel";
import { AccountUsageBar } from "./AccountUsageBar";

export const ENGINES_QUERY_KEY = ["review-engines"] as const;

const effortLabel: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  ultra: "Ultra",
};
const labelEffort = (effort: string) => effortLabel[effort] ?? effort;

/** The account, model and effort the next reviews run on, switchable from the top bar. */
export function EnginePicker() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const accounts = useQuery({ queryKey: ACCOUNTS_QUERY_KEY, queryFn: setupApi.listAccounts, refetchInterval: 60_000 });
  const engines = useQuery({ queryKey: ENGINES_QUERY_KEY, queryFn: setupApi.reviewEngines });
  const all = accounts.data ?? [];
  const usable = all.filter((account) => account.signedIn && account.enabled);
  const next = all.find((account) => account.id === nextAccountId(all));
  const engine = engines.data?.find((item) => item.provider === next?.provider);
  // New reviews start on the first ready account, so picking one moves it to the front.
  const useFirst = useMutation({
    mutationFn: (account: AiAccount) =>
      setupApi.reorderAccounts([
        account.id,
        ...all.filter((other) => other.id !== account.id).map((other) => other.id),
      ]),
    onSuccess: (data) => queryClient.setQueryData(ACCOUNTS_QUERY_KEY, data),
  });
  const save = useMutation({
    mutationFn: (choice: { provider: string; model: string; effort: string }) => setupApi.saveReviewEngine(choice),
    onSuccess: (data) => queryClient.setQueryData(ENGINES_QUERY_KEY, data),
  });
  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => {
      if (event instanceof KeyboardEvent ? event.key === "Escape" : !root.current?.contains(event.target as Node))
        setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [open]);
  if (!next || !engine) return null;
  const model = engine.models.find((item) => item.id === engine.model);
  const models = engine.models.filter((item) => item.label.toLowerCase().includes(search.trim().toLowerCase()));
  const chooseModel = (id: string) => {
    const efforts = engine.models.find((item) => item.id === id)?.efforts ?? [];
    save.mutate({ provider: engine.provider, model: id, effort: efforts.includes(engine.effort) ? engine.effort : "" });
  };
  // The trigger shows the model; effort and account are in its label and in the menu.
  const summary = [model?.label ?? engine.model, engine.effort && labelEffort(engine.effort), next.label]
    .filter(Boolean)
    .join(" · ");
  // Accounts of a provider with several are numbered in the rail, as they are tried.
  const number = (account: AiAccount) => {
    const same = usable.filter((other) => other.provider === account.provider);
    return same.length > 1 ? same.indexOf(account) + 1 : null;
  };
  return (
    <div ref={root} className="relative ml-auto flex items-center gap-3">
      <AccountUsageBar account={next} />
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={summary}
        title={`New reviews run on ${summary}`}
        className="flex h-8 max-w-[min(20rem,50vw)] cursor-pointer items-center gap-2 rounded-lg px-2 text-small text-fg transition-colors hover:bg-surface-muted"
        onClick={() => setOpen((value) => !value)}
      >
        <ProviderGlyph provider={engine.provider} />
        <span className="truncate font-medium">{model?.label ?? engine.model}</span>
        <ChevronDown
          className={cx("-ml-0.5 size-3.5 flex-none text-fg-subtle transition-transform", open && "rotate-180")}
        />
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Reviews run on"
          className="absolute top-full right-0 z-50 mt-2 flex max-h-[min(36rem,80vh)] w-[min(24rem,92vw)] overflow-hidden rounded-2xl border border-line-strong bg-surface shadow-pop"
        >
          <div className="flex w-14 flex-none flex-col items-center gap-1.5 overflow-y-auto border-r border-line bg-rail py-2.5">
            {usable.map((account) => {
              const selected = account.id === next.id;
              const n = number(account);
              return (
                <button
                  key={account.id}
                  type="button"
                  aria-label={`${providerName[account.provider]} · ${account.label}`}
                  aria-pressed={selected}
                  title={`${providerName[account.provider]} · ${account.label}`}
                  disabled={useFirst.isPending}
                  className={cx(
                    "relative grid size-10 flex-none cursor-pointer place-items-center rounded-xl transition-colors [&_svg]:size-5",
                    selected
                      ? "bg-surface-muted ring-1 ring-line-strong"
                      : "opacity-70 hover:bg-surface hover:opacity-100",
                  )}
                  onClick={() => !selected && useFirst.mutate(account)}
                >
                  <ProviderGlyph provider={account.provider} />
                  {n && (
                    <span className="absolute -right-0.5 -bottom-0.5 grid size-4 place-items-center rounded-full bg-canvas text-[0.625rem] font-semibold text-fg ring-1 ring-line-strong tabular-nums">
                      {n}
                    </span>
                  )}
                </button>
              );
            })}
            <Link
              to="/platform/settings"
              aria-label="Add an account"
              title="Add an account"
              className="grid size-10 flex-none place-items-center rounded-xl text-fg-subtle no-underline hover:bg-surface hover:text-fg"
              onClick={() => setOpen(false)}
            >
              <Plus className="size-4" />
            </Link>
          </div>
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="flex flex-col gap-2.5 px-4 pt-3.5 pb-2">
              <div className="min-w-0">
                <div className="truncate text-small font-semibold text-fg">
                  {providerName[engine.provider]} · {next.label}
                </div>
                <div className="truncate text-caption text-fg-subtle">New reviews start on this account</div>
              </div>
              {engine.models.length > 1 && (
                <input
                  aria-label="Search models"
                  placeholder="Search models"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  className="h-9 w-full rounded-lg bg-surface-muted px-3 text-small text-fg outline-none placeholder:text-fg-subtle focus:ring-2 focus:ring-brand/20"
                />
              )}
            </div>
            <div
              className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 pb-2"
              role="radiogroup"
              aria-label="Model"
            >
              {models.map((item) => {
                const selected = item.id === engine.model;
                return (
                  // biome-ignore lint/a11y/useSemanticElements: radio rows styled as a list; a native radio cannot hold this layout
                  <button
                    key={item.id}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    disabled={save.isPending}
                    className="flex h-9 w-full flex-none cursor-pointer items-center gap-2.5 rounded-lg px-2 text-left text-small text-fg transition-colors hover:bg-surface-muted/60"
                    onClick={() => !selected && chooseModel(item.id)}
                  >
                    <span
                      className={cx(
                        "grid size-4.5 flex-none place-items-center rounded-full border",
                        selected ? "border-fg bg-fg text-canvas" : "border-line-strong",
                      )}
                    >
                      {selected && <Check className="size-3" strokeWidth={3} />}
                    </span>
                    <span className="truncate">{item.label}</span>
                    {engine.models.length > 1 && item.id === engine.models[0].id && (
                      <span className="rounded-md bg-surface-muted px-1.5 py-0.5 text-caption text-fg-subtle">
                        default
                      </span>
                    )}
                  </button>
                );
              })}
              {models.length === 0 && <p className="px-2 py-3 text-caption text-fg-subtle">No model matches.</p>}
            </div>
            {model && model.efforts.length > 0 && (
              <div className="flex flex-col gap-2 border-t border-line px-4 py-3">
                <span className="text-caption text-fg-subtle">Effort</span>
                <div className="flex rounded-lg bg-surface-muted p-0.5">
                  {model.efforts.map((effort) => (
                    <button
                      key={effort}
                      type="button"
                      aria-pressed={effort === engine.effort}
                      disabled={save.isPending}
                      className={cx(
                        "h-8 min-w-0 flex-1 cursor-pointer truncate rounded-md px-1.5 text-caption font-medium transition-colors",
                        effort === engine.effort ? "bg-surface text-fg shadow-card" : "text-fg-muted hover:text-fg",
                      )}
                      onClick={() => save.mutate({ provider: engine.provider, model: engine.model, effort })}
                    >
                      {labelEffort(effort)}
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
