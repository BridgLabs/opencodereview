import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ChevronDown, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { BrandLoadingIcon } from "../../components/BrandLoading";
import { cx } from "../../components/DesignSystem";
import { setupApi } from "../../services/setupApi";
import type { AiAccount } from "../../types";
import { ACCOUNTS_QUERY_KEY, nextAccountId, ProviderGlyph, providerName, usageQuery } from "./AccountsPanel";
import { AccountUsageBar } from "./AccountUsageBar";
import { AccountChoices, AddAccount, engineCataloguePollInterval } from "./EngineAccounts";

export const ENGINES_QUERY_KEY = ["review-engines"] as const;

type Provider = AiAccount["provider"];

const effortLabel: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  ultra: "Ultra",
};
const labelEffort = (effort: string) => effortLabel[effort] ?? `${effort[0]?.toUpperCase() ?? ""}${effort.slice(1)}`;

/** The account, model and effort the next reviews run on, switchable from the top bar. */
export function EnginePicker() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  // The provider the menu shows, which starts as the next review's; null follows it.
  const [view, setView] = useState<Provider | null>(null);
  const [adding, setAdding] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const accounts = useQuery({ queryKey: ACCOUNTS_QUERY_KEY, queryFn: setupApi.listAccounts, refetchInterval: 60_000 });
  const engines = useQuery({
    queryKey: ENGINES_QUERY_KEY,
    queryFn: setupApi.reviewEngines,
    refetchInterval: (query) => engineCataloguePollInterval(query.state.data),
  });
  const all = accounts.data ?? [];
  const next = all.find((account) => account.id === nextAccountId(all));
  const engine = engines.data?.find((item) => item.provider === next?.provider);
  // New reviews start on the first ready account, so choosing one moves it to the front, turning
  // it back on first if it was paused.
  const choose = useMutation({
    mutationFn: async (account: AiAccount) => {
      if (!account.enabled) await setupApi.updateAccount(account.id, { enabled: true });
      return setupApi.reorderAccounts([
        account.id,
        ...all.filter((other) => other.id !== account.id).map((other) => other.id),
      ]);
    },
    onSuccess: (data) => queryClient.setQueryData(ACCOUNTS_QUERY_KEY, data),
  });
  const save = useMutation({
    mutationFn: (choice: { provider: Provider; model: string; effort: string; catalogueId?: string }) =>
      setupApi.saveReviewEngine(choice),
    onSuccess: (data) => queryClient.setQueryData(ENGINES_QUERY_KEY, data),
  });
  const refreshCatalogue = useMutation({
    mutationFn: setupApi.refreshOpenCodeCatalogue,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ENGINES_QUERY_KEY }),
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
  const engineValid =
    engine.provider !== "opencode" ||
    (engine.catalogue?.status === "ready" && Boolean(engine.catalogue.id) && engine.selectionValid === true);
  const model = engineValid ? engine.models.find((item) => item.id === engine.model) : undefined;
  const enginePrompt =
    engine.provider !== "opencode" || engineValid
      ? (model?.label ?? engine.model)
      : engine.catalogue?.status === "error" || (engine.catalogue?.status === "ready" && !engine.catalogue.id)
        ? "OpenCode Go models unavailable"
        : engine.catalogue?.status === "ready"
          ? "Choose an OpenCode Go model"
          : "Loading OpenCode Go models";
  // The trigger shows the model; effort and account are in its label and in the menu.
  const summary = [enginePrompt, engineValid && engine.effort && labelEffort(engine.effort), next.label]
    .filter(Boolean)
    .join(" · ");
  const provider = view ?? next.provider;
  const shown = engines.data?.find((item) => item.provider === provider) ?? engine;
  const catalogueStatus = shown.provider === "opencode" ? shown.catalogue?.status : "ready";
  const catalogueReady = shown.provider !== "opencode" || (catalogueStatus === "ready" && Boolean(shown.catalogue?.id));
  const catalogueFailed =
    shown.provider === "opencode" &&
    (catalogueStatus === "error" || (catalogueStatus === "ready" && !shown.catalogue?.id));
  const shownSelectionValid = shown.provider !== "opencode" || (catalogueReady && shown.selectionValid === true);
  const shownModel = shownSelectionValid ? shown.models.find((item) => item.id === shown.model) : undefined;
  const models = catalogueReady
    ? shown.models.filter((item) => item.label.toLowerCase().includes(search.trim().toLowerCase()))
    : [];
  const chooseModel = (id: string) => {
    const selected = shown.models.find((item) => item.id === id);
    const catalogueId = shown.provider === "opencode" ? shown.catalogue?.id : undefined;
    if (!selected || (shown.provider === "opencode" && (!catalogueReady || !catalogueId))) return;
    const efforts = selected?.efforts ?? [];
    save.mutate({
      provider: shown.provider,
      model: id,
      effort: shownSelectionValid && efforts.includes(shown.effort) ? shown.effort : (selected.defaultEffort ?? ""),
      ...(catalogueId ? { catalogueId } : {}),
    });
  };
  const saveEffort = (effort: string) => {
    const catalogueId = shown.provider === "opencode" ? shown.catalogue?.id : undefined;
    if (shown.provider === "opencode" && (!catalogueReady || !catalogueId)) return;
    save.mutate({
      provider: shown.provider,
      model: shown.model,
      effort,
      ...(catalogueId ? { catalogueId } : {}),
    });
  };
  const connected = [...new Set(all.map((account) => account.provider))];
  const added = (item: Provider) => {
    setAdding(false);
    setView(item);
    void queryClient.invalidateQueries({ queryKey: ACCOUNTS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: usageQuery.queryKey });
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
        onClick={() => {
          setView(null);
          setAdding(false);
          setOpen(!open);
        }}
      >
        <ProviderGlyph provider={engine.provider} />
        <span className="truncate font-medium">{engineValid ? (model?.label ?? engine.model) : enginePrompt}</span>
        <ChevronDown
          className={cx("-ml-0.5 size-3.5 flex-none text-fg-subtle transition-transform", open && "rotate-180")}
        />
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Reviews run on"
          className="absolute top-full right-0 z-50 mt-2 flex max-h-[min(40rem,85vh)] w-[min(24rem,92vw)] overflow-hidden rounded-2xl border border-line-strong bg-surface shadow-pop"
        >
          <div className="flex w-14 flex-none flex-col items-center gap-1.5 overflow-y-auto border-r border-line bg-rail py-2.5">
            {connected.map((item) => (
              <button
                key={item}
                type="button"
                aria-label={providerName[item]}
                aria-pressed={!adding && item === provider}
                title={providerName[item]}
                className={cx(
                  "relative grid size-10 flex-none cursor-pointer place-items-center rounded-xl transition-colors [&_svg]:size-5",
                  !adding && item === provider
                    ? "bg-surface-muted ring-1 ring-line-strong"
                    : "opacity-70 hover:bg-surface hover:opacity-100",
                )}
                onClick={() => {
                  setAdding(false);
                  setSearch("");
                  setView(item);
                }}
              >
                <ProviderGlyph provider={item} />
              </button>
            ))}
            <button
              type="button"
              aria-label="Add an account"
              aria-pressed={adding}
              title="Add an account"
              className={cx(
                "grid size-10 flex-none cursor-pointer place-items-center rounded-xl text-fg-subtle hover:bg-surface hover:text-fg",
                adding && "bg-surface-muted text-fg ring-1 ring-line-strong",
              )}
              onClick={() => setAdding(true)}
            >
              <Plus className="size-4" />
            </button>
          </div>
          {adding ? (
            <AddAccount onDone={added} onCancel={() => setAdding(false)} />
          ) : (
            <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
              <div className="px-4 pt-3.5 pb-1.5">
                <div className="truncate text-small font-semibold text-fg">{providerName[provider]}</div>
                <div className="truncate text-caption text-fg-subtle">
                  {provider === "opencode" && !catalogueReady
                    ? catalogueFailed
                      ? "OpenCode Go models need a refresh"
                      : "Loading OpenCode Go models…"
                    : provider === "opencode" && !shownSelectionValid
                      ? "Choose a model before reviews can use this account"
                      : provider === next.provider
                        ? "New reviews start on the chosen account"
                        : `New reviews run on ${providerName[next.provider]} · ${next.label}`}
                </div>
              </div>
              <AccountChoices
                accounts={all.filter((account) => account.provider === provider)}
                nextId={shownSelectionValid ? next.id : undefined}
                busy={choose.isPending || (provider === "opencode" && !shownSelectionValid)}
                onChoose={(account) => choose.mutate(account)}
              />
              <div className="flex flex-col gap-2.5 border-t border-line px-4 pt-3 pb-2">
                <span className="text-caption text-fg-subtle">Model</span>
                {catalogueReady && shown.models.length > 1 && (
                  <input
                    aria-label="Search models"
                    placeholder="Search models"
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                    className="h-9 w-full rounded-lg bg-surface-muted px-3 text-small text-fg outline-none placeholder:text-fg-subtle focus:ring-2 focus:ring-brand/20"
                  />
                )}
              </div>
              <div className="flex flex-col px-2 pb-2" role="radiogroup" aria-label="Model">
                {shown.provider === "opencode" &&
                  (catalogueStatus === "empty" || catalogueStatus === "loading" || !catalogueStatus) && (
                    <p className="flex items-center gap-2 px-2 py-3 text-caption text-fg-muted" role="status">
                      <BrandLoadingIcon size={16} /> Loading OpenCode Go models…
                    </p>
                  )}
                {catalogueFailed && (
                  <div className="flex flex-col gap-2 px-2 py-3" role="alert">
                    <p className="text-caption text-danger">
                      {shown.catalogue?.error || "Could not load OpenCode Go models."}
                      {refreshCatalogue.error && " Refresh failed. Try again."}
                    </p>
                    <button
                      type="button"
                      className="cursor-pointer self-start text-caption font-medium text-fg underline-offset-2 hover:underline"
                      disabled={refreshCatalogue.isPending}
                      onClick={() => refreshCatalogue.mutate()}
                    >
                      {refreshCatalogue.isPending ? "Retrying…" : "Retry"}
                    </button>
                  </div>
                )}
                {shown.provider === "opencode" && catalogueReady && !shownSelectionValid && (
                  <p className="px-2 py-2 text-caption text-warning">Choose a model to enable reviews.</p>
                )}
                {shown.provider === "opencode" && catalogueReady && shown.models.length === 0 && (
                  <div className="flex flex-col gap-2 px-2 py-3">
                    <p className="text-caption text-fg-muted">No OpenCode Go models are available yet.</p>
                    <button
                      type="button"
                      className="cursor-pointer self-start text-caption font-medium text-fg underline-offset-2 hover:underline"
                      disabled={refreshCatalogue.isPending}
                      onClick={() => refreshCatalogue.mutate()}
                    >
                      {refreshCatalogue.isPending ? "Refreshing…" : "Refresh models"}
                    </button>
                  </div>
                )}
                {models.map((item) => {
                  const selected = shownSelectionValid && item.id === shown.model;
                  return (
                    // biome-ignore lint/a11y/useSemanticElements: radio rows styled as a list; a native radio cannot hold this layout
                    <button
                      key={item.id}
                      type="button"
                      role="radio"
                      aria-checked={selected}
                      disabled={save.isPending || (shown.provider === "opencode" && !shown.catalogue?.id)}
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
                      {shown.provider !== "opencode" && shown.models.length > 1 && item.id === shown.models[0].id && (
                        <span className="rounded-md bg-surface-muted px-1.5 py-0.5 text-caption text-fg-subtle">
                          default
                        </span>
                      )}
                    </button>
                  );
                })}
                {catalogueReady && models.length === 0 && shown.models.length > 0 && (
                  <p className="px-2 py-3 text-caption text-fg-subtle">No model matches.</p>
                )}
              </div>
              {shownModel && shownModel.efforts.length > 0 && (
                <div className="flex flex-col gap-2 border-t border-line px-4 py-3">
                  <span className="text-caption text-fg-subtle">Effort</span>
                  <div className="grid grid-cols-3 gap-0.5 rounded-lg bg-surface-muted p-0.5">
                    {shownModel.efforts.map((effort) => (
                      <button
                        key={effort}
                        type="button"
                        aria-pressed={effort === shown.effort}
                        disabled={save.isPending}
                        className={cx(
                          "h-8 min-w-0 cursor-pointer rounded-md px-1.5 text-caption font-medium transition-colors",
                          effort === shown.effort ? "bg-surface text-fg shadow-card" : "text-fg-muted hover:text-fg",
                        )}
                        onClick={() => saveEffort(effort)}
                      >
                        {labelEffort(effort)}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
