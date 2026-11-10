import type {
  AccountUsage,
  AiAccount,
  AiProvider,
  GitHubAuthStatus,
  IndexGraph,
  Repo,
  RepoIndexStatus,
  ReviewEngine,
  Settings,
  Workspace,
} from "../types";
import type { IndexJobsResponse } from "./setupApiTransport";
import { http } from "./setupApiTransport";

let githubFlowSecret: string | null = sessionStorage.getItem("ocr.githubFlowSecret");
let claudeFlowSecret: string | null = sessionStorage.getItem("ocr.claudeFlowSecret");
let codexFlowSecret: string | null = sessionStorage.getItem("ocr.codexFlowSecret");
let opencodeFlowSecret: string | null = sessionStorage.getItem("ocr.opencodeFlowSecret");
const opencodeHeaders = () => ({
  "content-type": "application/json",
  ...(opencodeFlowSecret ? { "x-flow-secret": opencodeFlowSecret } : {}),
});
export const setupApiAccess = {
  githubAuthStatus: () => http<GitHubAuthStatus>("/auth/github/status"),
  githubAuthStart: async (action: "login" | "refresh" = "login") => {
    const result = await http<{ running: boolean; flowSecret?: string }>("/auth/github/start", {
      method: "POST",
      body: JSON.stringify({ action }),
    });
    if (result.flowSecret) {
      githubFlowSecret = result.flowSecret;
      sessionStorage.setItem("ocr.githubFlowSecret", result.flowSecret);
    }
  },
  githubAuthEvents: () => {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (githubFlowSecret) headers["x-flow-secret"] = githubFlowSecret;
    return http<{
      running: boolean;
      events: Array<{ kind: string; code?: string; url?: string; success?: boolean; message?: string }>;
    }>("/auth/github/events", { headers });
  },
  listAccounts: () => http<AiAccount[]>("/accounts"),
  accountUsage: () => http<AccountUsage[]>("/accounts/usage"),
  updateAccount: (id: number, update: { label?: string; enabled?: boolean }) =>
    http<AiAccount[]>(`/accounts/${id}`, { method: "POST", body: JSON.stringify(update) }),
  reorderAccounts: (ids: number[]) =>
    http<AiAccount[]>("/accounts/order", { method: "POST", body: JSON.stringify({ ids }) }),
  deleteAccount: (id: number) => http<AiAccount[]>(`/accounts/${id}`, { method: "DELETE" }),
  reviewEngines: () => http<ReviewEngine[]>("/review-engines"),
  saveReviewEngine: (engine: { provider: string; model: string; effort: string }) =>
    http<ReviewEngine[]>("/review-engines", { method: "POST", body: JSON.stringify(engine) }),
  claudeAuthStatus: () => http<{ loggedIn: boolean; message: string; version?: string | null }>("/auth/claude/status"),
  claudeAuthStart: async (accountId?: number) => {
    await http<{ running: boolean }>("/auth/claude/cancel", { method: "POST", body: JSON.stringify({}) }).catch(
      () => undefined,
    );
    claudeFlowSecret = null;
    sessionStorage.removeItem("ocr.claudeFlowSecret");
    const result = await http<{ running: boolean; flowSecret?: string }>("/auth/claude/start", {
      method: "POST",
      body: JSON.stringify(accountId ? { accountId } : {}),
    });
    if (result.flowSecret) {
      claudeFlowSecret = result.flowSecret;
      sessionStorage.setItem("ocr.claudeFlowSecret", result.flowSecret);
    }
  },
  claudeAuthEvents: () => {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (claudeFlowSecret) headers["x-flow-secret"] = claudeFlowSecret;
    return http<{
      running: boolean;
      events: Array<{ kind: string; code?: string; url?: string; success?: boolean; message?: string }>;
    }>("/auth/claude/events", { headers });
  },
  claudeAuthInput: async (input: string) => {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (claudeFlowSecret) headers["x-flow-secret"] = claudeFlowSecret;
    try {
      return await http<{ ok: boolean }>("/auth/claude/input", {
        method: "POST",
        headers,
        body: JSON.stringify({ input }),
      });
    } catch (error) {
      if (error instanceof Error && error.message.includes("not running")) {
        claudeFlowSecret = null;
        sessionStorage.removeItem("ocr.claudeFlowSecret");
      }
      throw error;
    }
  },
  claudeAuthCancel: () =>
    http<{ running: boolean }>("/auth/claude/cancel", { method: "POST", body: JSON.stringify({}) }),
  aiProviders: () => http<AiProvider[]>("/ai/providers"),
  claudeTokenSave: (token: string, accountId?: number) =>
    http<{ ok: boolean }>("/auth/claude/token", {
      method: "POST",
      body: JSON.stringify(accountId ? { token, accountId } : { token }),
    }),
  codexAuthStart: async (accountId?: number, mode: "browser" | "device" = "browser") => {
    await http<{ running: boolean }>("/auth/codex/cancel", { method: "POST", body: JSON.stringify({}) }).catch(
      () => undefined,
    );
    codexFlowSecret = null;
    sessionStorage.removeItem("ocr.codexFlowSecret");
    const result = await http<{ running: boolean; flowSecret?: string }>("/auth/codex/start", {
      method: "POST",
      body: JSON.stringify({ ...(accountId ? { accountId } : {}), mode }),
    });
    if (result.flowSecret) {
      codexFlowSecret = result.flowSecret;
      sessionStorage.setItem("ocr.codexFlowSecret", result.flowSecret);
    }
  },
  codexAuthEvents: () => {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (codexFlowSecret) headers["x-flow-secret"] = codexFlowSecret;
    return http<{
      running: boolean;
      events: Array<{ kind: string; code?: string; url?: string; success?: boolean; message?: string }>;
    }>("/auth/codex/events", { headers });
  },
  codexAuthCancel: () => {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (codexFlowSecret) headers["x-flow-secret"] = codexFlowSecret;
    return http<{ running: boolean }>("/auth/codex/cancel", { method: "POST", headers, body: JSON.stringify({}) });
  },
  /** Signs in to the OpenCode console with a one-time code, saving it to `accountId` or to a new account. */
  opencodeAuthStart: async (accountId?: number) => {
    await http<{ running: boolean }>("/auth/opencode/cancel", {
      method: "POST",
      headers: opencodeHeaders(),
      body: JSON.stringify({}),
    }).catch(() => undefined);
    opencodeFlowSecret = null;
    sessionStorage.removeItem("ocr.opencodeFlowSecret");
    const result = await http<{ running: boolean; flowSecret?: string }>("/auth/opencode/start", {
      method: "POST",
      body: JSON.stringify(accountId ? { accountId } : {}),
    });
    if (result.flowSecret) {
      opencodeFlowSecret = result.flowSecret;
      sessionStorage.setItem("ocr.opencodeFlowSecret", result.flowSecret);
    }
  },
  opencodeAuthEvents: () =>
    http<{
      running: boolean;
      events: Array<{ kind: string; code?: string; url?: string; success?: boolean; message?: string }>;
    }>("/auth/opencode/events", { headers: opencodeHeaders() }),
  opencodeAuthCancel: () =>
    http<{ running: boolean }>("/auth/opencode/cancel", {
      method: "POST",
      headers: opencodeHeaders(),
      body: JSON.stringify({}),
    }),
  /** Saves an OpenCode API key, once OpenCode accepts it, to `accountId` or to a new account. */
  opencodeKeySave: (key: string, accountId?: number) =>
    http<{ ok: boolean }>("/auth/opencode/key", {
      method: "POST",
      body: JSON.stringify({ key, ...(accountId ? { accountId } : {}) }),
    }),
  githubAuthLogout: () => http<{ ok: boolean }>("/auth/github/logout", { method: "POST", body: JSON.stringify({}) }),
  getSettings: () => http<Settings>("/settings"),
  saveSettingsBatch: (payload: {
    settings: Record<string, string>;
    repositories?: { logins: string[]; repoIds: number[]; monitorAll: string[] };
  }) =>
    http<{ settings: Settings; workspaces: Workspace[]; selectedRepos: Repo[] }>("/settings/batch", {
      method: "POST",
      body: JSON.stringify(payload),
    }),
  listIndexJobs: () => http<IndexJobsResponse>("/indexing/jobs"),
  reindexRepo: (repoId: number) => http<RepoIndexStatus>(`/repos/${repoId}/index`, { method: "POST" }),
  indexGraph: (repoId: number) => http<IndexGraph>(`/repos/${repoId}/index/graph`),
  indexPackageFiles: (repoId: number, packageId: number) =>
    http<IndexGraph>(`/repos/${repoId}/index/packages/${packageId}/graph`),
  symbolGraph: (repoId: number) => http<IndexGraph>(`/repos/${repoId}/index/symbol-graph`),
};
