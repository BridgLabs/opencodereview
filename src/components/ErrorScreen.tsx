import { type ErrorComponentProps, useRouter } from "@tanstack/react-router";
import { Check, Copy, RotateCcw } from "lucide-react";
import { useState } from "react";
import { BrandMark } from "./BrandMark";
import { DSButton, DSLinkButton } from "./DesignSystem";

export function ErrorScreen({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  const [copied, setCopied] = useState(false);
  const message = error instanceof Error ? error.message : String(error);
  const details = [
    message,
    error instanceof Error ? error.stack?.split("\n").slice(1, 8).join("\n") : null,
    `Page: ${window.location.pathname}${window.location.search}`,
  ]
    .filter(Boolean)
    .join("\n");
  const retry = () => {
    reset();
    void router.invalidate();
  };
  const copy = () => void navigator.clipboard?.writeText(details).then(() => setCopied(true));
  return (
    <div role="alert" className="flex min-h-0 flex-1 items-start justify-center overflow-y-auto px-6 py-16">
      <div className="flex w-full max-w-xl flex-col items-center gap-5 text-center">
        <BrandMark size="md" />
        <div className="flex flex-col gap-1.5">
          <p className="text-overline text-danger">Something broke on this page</p>
          <h1 className="text-title text-fg">This page couldn’t be shown</h1>
          <p className="text-small text-fg-muted">
            Your data is safe: nothing was changed or lost. Try again, or open another page. If it keeps happening after
            reloading, the web app and the API may be on different versions; restarting OpenCodeReview usually fixes
            that.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <DSLinkButton to="/platform/reviews">Go to Reviews</DSLinkButton>
          <DSButton tone="primary" onClick={retry}>
            <RotateCcw className="size-3.5" />
            Try again
          </DSButton>
        </div>
        <details className="group w-full rounded-xl border border-line bg-surface text-left shadow-card">
          <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-4 py-3 text-small font-medium text-fg-muted select-none hover:text-fg [&::-webkit-details-marker]:hidden">
            Technical details<span className="text-caption text-fg-subtle group-open:hidden">Show</span>
            <span className="hidden text-caption text-fg-subtle group-open:inline">Hide</span>
          </summary>
          <div className="relative border-t border-line">
            <pre className="max-h-60 overflow-auto p-4 pr-24 font-mono text-caption leading-relaxed whitespace-pre-wrap text-rose-700 [overflow-wrap:anywhere]">
              {details}
            </pre>
            <DSButton size="sm" className="absolute top-2.5 right-2.5" onClick={copy}>
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
              {copied ? "Copied" : "Copy"}
            </DSButton>
          </div>
        </details>
      </div>
    </div>
  );
}
