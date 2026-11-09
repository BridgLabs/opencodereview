import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { friendlyError } from "../../components/AccessConnect";
import { BrandLoadingIcon } from "../../components/BrandLoading";
import { cx, DSButton } from "../../components/DesignSystem";
import { setupApi } from "../../services/setupApi";
import type { AiAccount, ReviewEngine } from "../../types";
import { ACCOUNTS_QUERY_KEY, AccountsPanel, accountHealth, ProviderGlyph, providerName } from "./AccountsPanel";
import { engineCataloguePollInterval } from "./EngineAccounts";
import { ENGINES_QUERY_KEY } from "./EnginePicker";

type Provider = AiAccount["provider"];
const planHint: Record<Provider, string> = {
  claude: "Claude Pro, Max or Team plan",
  codex: "ChatGPT Plus or Pro plan",
  opencode: "OpenCode Go API key",
};

const groupClass = "flex flex-col overflow-hidden rounded-xl border border-line/70 bg-surface";
const rowClass =
  "flex w-full cursor-pointer items-center gap-3 border-b border-line px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-surface-muted/50";

/** Providers in the order reviews try them: by their first account's place in the account list. */
function connectedProviders(accounts: AiAccount[]) {
  return [...new Set(accounts.map((account) => account.provider))].sort(
    (a, b) =>
      accounts.findIndex((account) => account.provider === a) - accounts.findIndex((account) => account.provider === b),
  );
}

function useMoveFirst() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ provider, accounts }: { provider: Provider; accounts: AiAccount[] }) =>
      setupApi.reorderAccounts([
        ...accounts.filter((account) => account.provider === provider).map((account) => account.id),
        ...accounts.filter((account) => account.provider !== provider).map((account) => account.id),
      ]),
    onSuccess: (data) => queryClient.setQueryData(ACCOUNTS_QUERY_KEY, data),
  });
}

/** Settings → Providers: the AI providers reviews run on, with each one's accounts, model and effort. */
export function ProvidersPanel() {
  const [open, setOpen] = useState<Provider | null>(null);
  const accountsQuery = useQuery({ queryKey: ACCOUNTS_QUERY_KEY, queryFn: setupApi.listAccounts });
  const engines = useQuery({
    queryKey: ENGINES_QUERY_KEY,
    queryFn: setupApi.reviewEngines,
    refetchInterval: (query) => engineCataloguePollInterval(query.state.data),
  });
  const moveFirst = useMoveFirst();
  const accounts = accountsQuery.data ?? [];
  const connected = connectedProviders(accounts);
  const runnable = connected.filter((provider) => {
    const engine = engines.data?.find((item) => item.provider === provider);
    return (
      provider !== "opencode" ||
      (engine?.catalogue?.status === "ready" && Boolean(engine.catalogue.id) && engine.selectionValid === true)
    );
  });
  const others = (["claude", "codex", "opencode"] as Provider[]).filter((provider) => !connected.includes(provider));
  const engineLabel = (provider: Provider) =>
    (() => {
      const engine = engines.data?.find((item) => item.provider === provider);
      if (
        provider === "opencode" &&
        engine?.catalogue?.status === "ready" &&
        (!engine.catalogue.id || engine.selectionValid !== true)
      )
        return "Choose model";
      if (provider === "opencode" && engine?.catalogue?.status === "error") return "Models unavailable";
      if (provider === "opencode" && engines.error) return "Models unavailable";
      if (provider === "opencode" && engine?.catalogue?.status !== "ready") return "Loading models…";
      if (provider === "opencode") {
        const model = engine?.models.find((item) => item.id === engine.model);
        return [model?.label, engine?.effort].filter(Boolean).join(" · ");
      }
      return engine?.label.split(" · ").slice(1).join(" · ");
    })();
  if (open)
    return (
      <ProviderDetail
        provider={open}
        accounts={accounts}
        engine={engines.data?.find((engine) => engine.provider === open)}
        discoveryError={engines.error}
        retryDiscovery={() => void engines.refetch()}
        first={connected[0] === open && runnable.includes(open)}
        onBack={() => setOpen(null)}
      />
    );
  const summary = (provider: Provider) => {
    const mine = accounts.filter((account) => account.provider === provider);
    const signedOut = mine.filter((account) => accountHealth(account) === "signed-out").length;
    const who = mine.length === 1 ? mine[0].label : `${mine.length} accounts`;
    return [who, engineLabel(provider), signedOut ? `${signedOut} signed out` : ""].filter(Boolean).join(" · ");
  };
  return (
    <section className="settings-group flex flex-col gap-5">
      <header className="px-1">
        <h2 className="text-small font-semibold text-fg">Providers</h2>
        <p className="mt-0.5 max-w-2xl text-caption text-fg-subtle">
          Reviews use connected accounts or API keys. If one account cannot continue, reviews can move to the next.
        </p>
      </header>
      {connected.length > 0 && (
        <div className={cx(groupClass, "px-4 py-3")}>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div className="text-small font-medium text-fg">Reviews run on</div>
              <div className="text-caption text-fg-subtle">New reviews start with this provider.</div>
            </div>
            <select
              aria-label="Reviews run on"
              className="h-9 rounded-lg border border-line bg-surface-muted px-3 text-small text-fg outline-none focus:border-line-strong"
              value={connected[0]}
              disabled={moveFirst.isPending}
              onChange={(event) => moveFirst.mutate({ provider: event.target.value as Provider, accounts })}
            >
              {connected.map((provider) => (
                <option
                  key={provider}
                  value={provider}
                  disabled={provider === "opencode" && !runnable.includes(provider)}
                >
                  {providerName[provider]}
                  {engineLabel(provider) ? ` · ${engineLabel(provider)}` : ""}
                </option>
              ))}
            </select>
          </div>
        </div>
      )}
      {connected.length > 0 && (
        <ProviderGroup title="Connected">
          {connected.map((provider) => {
            const requiresModel = provider === "opencode" && !runnable.includes(provider);
            return (
              <button key={provider} type="button" className={rowClass} onClick={() => setOpen(provider)}>
                <ProviderGlyph provider={provider} />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2 text-small font-medium text-fg">
                    {providerName[provider]}
                    {connected[0] === provider && !requiresModel && runnable.includes(provider) && (
                      <span className="rounded-full bg-brand-soft px-2 py-0.5 text-caption font-semibold text-brand-hover">
                        Next review
                      </span>
                    )}
                    {requiresModel && (
                      <span className="rounded-full bg-warning/15 px-2 py-0.5 text-caption font-semibold text-warning">
                        Model required
                      </span>
                    )}
                  </span>
                  <span className="block truncate text-caption text-fg-subtle">{summary(provider)}</span>
                </span>
                <ChevronRight className="size-4 flex-none text-fg-subtle" />
              </button>
            );
          })}
        </ProviderGroup>
      )}
      {others.length > 0 && (
        <ProviderGroup title={connected.length ? "Other providers" : "Connect a provider"}>
          {others.map((provider) => (
            <button key={provider} type="button" className={rowClass} onClick={() => setOpen(provider)}>
              <ProviderGlyph provider={provider} />
              <span className="min-w-0 flex-1">
                <span className="block text-small font-medium text-fg">{providerName[provider]}</span>
                <span className="block truncate text-caption text-fg-subtle">{planHint[provider]}</span>
              </span>
              <ChevronRight className="size-4 flex-none text-fg-subtle" />
            </button>
          ))}
        </ProviderGroup>
      )}
    </section>
  );
}

function ProviderGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-2">
      <h3 className="px-1 text-caption font-medium text-fg-muted">{title}</h3>
      <div className={groupClass}>{children}</div>
    </div>
  );
}

function ProviderDetail({
  provider,
  accounts,
  engine,
  discoveryError,
  retryDiscovery,
  first,
  onBack,
}: {
  provider: Provider;
  accounts: AiAccount[];
  engine?: ReviewEngine;
  discoveryError: Error | null;
  retryDiscovery: () => void;
  first: boolean;
  onBack: () => void;
}) {
  const queryClient = useQueryClient();
  const moveFirst = useMoveFirst();
  const go = provider === "opencode";
  const catalogue = engine?.catalogue;
  const catalogueReady = !go || (catalogue?.status === "ready" && Boolean(catalogue.id));
  const catalogueFailed = go && (catalogue?.status === "error" || (catalogue?.status === "ready" && !catalogue.id));
  const selectionValid = !go || (catalogueReady && Boolean(catalogue?.id) && engine?.selectionValid === true);
  const save = useMutation({
    mutationFn: (choice: { model: string; effort: string; catalogueId?: string }) =>
      setupApi.saveReviewEngine({ provider, ...choice }),
    onSuccess: (data) => queryClient.setQueryData(ENGINES_QUERY_KEY, data),
    onError: () => void queryClient.invalidateQueries({ queryKey: ENGINES_QUERY_KEY }),
  });
  const refresh = useMutation({
    mutationFn: setupApi.refreshOpenCodeCatalogue,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ENGINES_QUERY_KEY }),
  });
  const connected = accounts.some((account) => account.provider === provider);
  const model = engine && selectionValid ? engine.models.find((item) => item.id === engine.model) : undefined;
  const saveChoice = (modelId: string, effort: string) => {
    const catalogueId = go ? catalogue?.id : undefined;
    if (go && (!catalogueReady || !catalogueId)) return;
    save.mutate({
      model: modelId,
      effort,
      ...(catalogueId ? { catalogueId } : {}),
    });
  };
  const retryButton = (label = "Retry") => (
    <DSButton size="sm" disabled={refresh.isPending} onClick={() => refresh.mutate()}>
      {refresh.isPending ? "Refreshing…" : label}
    </DSButton>
  );
  return (
    <section className="settings-group flex flex-col gap-5">
      <nav className="flex items-center gap-1.5 px-1 text-small" aria-label="Breadcrumb">
        <button type="button" className="cursor-pointer text-fg-subtle hover:text-fg" onClick={onBack}>
          Providers
        </button>
        <ChevronRight className="size-3.5 text-fg-subtle" />
        <span className="font-semibold text-fg">{providerName[provider]}</span>
      </nav>
      <header className="flex flex-wrap items-center gap-3 px-1">
        <span className="grid size-10 place-items-center rounded-xl border border-line bg-surface">
          <ProviderGlyph provider={provider} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-heading text-fg">
            {providerName[provider]}
            {connected && (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-500/10 px-2 py-0.5 text-caption font-medium text-emerald-500">
                <span className="size-1.5 rounded-full bg-emerald-500" />
                Connected
              </span>
            )}
          </div>
          <div className="text-caption text-fg-subtle">
            {go && !catalogueReady
              ? catalogueFailed
                ? "OpenCode Go models need a refresh"
                : discoveryError
                  ? friendlyError(discoveryError)
                  : "Loading OpenCode Go models…"
              : go && !selectionValid
                ? "Choose a model before reviews can use this account"
                : first
                  ? "New reviews start here"
                  : planHint[provider]}
          </div>
        </div>
        {connected && !first && (!go || selectionValid) && (
          <DSButton disabled={moveFirst.isPending} onClick={() => moveFirst.mutate({ provider, accounts })}>
            Use for new reviews
          </DSButton>
        )}
      </header>
      <AccountsPanel provider={provider} engineValid={selectionValid} />
      {discoveryError && (
        <div className={cx(groupClass, "flex flex-wrap items-center justify-between gap-3 px-4 py-3")} role="alert">
          <p className="text-small text-danger">
            {friendlyError(discoveryError) || "Could not load review engines."} Try again.
          </p>
          <DSButton size="sm" onClick={retryDiscovery}>
            Retry
          </DSButton>
        </div>
      )}
      {engine && (
        <div className="flex flex-col gap-2">
          <h3 className="px-1 text-small font-semibold text-fg">Model</h3>
          {provider === "opencode" && (
            <p className="px-1 text-caption text-fg-subtle">
              Review code and findings are sent to OpenCode Go for review.
            </p>
          )}
          <div className={cx(groupClass, "divide-y divide-line")}>
            {save.error && (
              <p className="px-4 py-3 text-small text-danger" role="alert">
                {save.error.message}
              </p>
            )}
            <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
              <div>
                <div className="text-small font-medium text-fg">Model</div>
                <div className="text-caption text-fg-subtle">The model reviews run on with this provider.</div>
              </div>
              {catalogueReady && engine.models.length > 0 && (engine.models.length > 1 || (go && !selectionValid)) && (
                <select
                  aria-label={`${providerName[provider]} model`}
                  className="h-9 rounded-lg border border-line bg-surface-muted px-3 text-small text-fg outline-none focus:border-line-strong"
                  value={selectionValid ? engine.model : ""}
                  disabled={save.isPending || (go && !catalogue?.id)}
                  onChange={(event) => {
                    const next = engine.models.find((item) => item.id === event.target.value);
                    if (!next) return;
                    saveChoice(
                      event.target.value,
                      selectionValid && next.efforts.includes(engine.effort)
                        ? engine.effort
                        : (next.defaultEffort ?? ""),
                    );
                  }}
                >
                  {go && !selectionValid && (
                    <option value="" disabled>
                      Choose a model…
                    </option>
                  )}
                  {engine.models.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.label}
                    </option>
                  ))}
                </select>
              )}
            </div>
            {go &&
              !discoveryError &&
              (!catalogue || catalogue.status === "empty" || catalogue.status === "loading") && (
                <div className="flex items-center gap-2 px-4 py-3 text-small text-fg-muted" role="status">
                  <BrandLoadingIcon size={16} /> Loading OpenCode Go models…
                </div>
              )}
            {catalogueFailed && (
              <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3" role="alert">
                <p className="text-small text-danger">
                  {catalogue?.error || "Could not load OpenCode Go models."}
                  {refresh.error && " Refresh failed. Try again."}
                </p>
                {retryButton()}
              </div>
            )}
            {go && catalogueReady && engine.models.length === 0 && (
              <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                <p className="text-small text-fg-muted">No OpenCode Go models are available yet.</p>
                {retryButton("Refresh models")}
              </div>
            )}
            {go && catalogueReady && !selectionValid && engine.models.length > 0 && (
              <p className="px-4 py-3 text-small text-warning" role="status">
                Choose a model to enable reviews.
              </p>
            )}
            {model && model.efforts.length > 0 && (
              <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                <div>
                  <div className="text-small font-medium text-fg">Effort</div>
                  <div className="text-caption text-fg-subtle">
                    {go
                      ? "More effort can use more tokens for the review."
                      : "More effort finds subtler issues and uses more of your plan."}
                  </div>
                </div>
                <fieldset className="flex flex-wrap gap-1" aria-label="Effort">
                  {model.efforts.map((effort) => (
                    <button
                      key={effort}
                      type="button"
                      aria-pressed={effort === engine.effort}
                      disabled={save.isPending}
                      className={cx(
                        "h-8 cursor-pointer rounded-md px-3 text-caption font-medium capitalize transition-colors",
                        effort === engine.effort ? "bg-fg text-canvas" : "bg-surface-muted text-fg-muted hover:text-fg",
                      )}
                      onClick={() => saveChoice(engine.model, effort)}
                    >
                      {effort}
                    </button>
                  ))}
                </fieldset>
              </div>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
