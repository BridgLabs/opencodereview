import type { ReactNode } from "react";
import { DSButton } from "../../../components/DesignSystem";
import type { SettingsSectionId } from "./settingsForm";

export function SettingsSection({
  id,
  activeSection,
  children,
}: {
  id: SettingsSectionId;
  activeSection: SettingsSectionId;
  children: ReactNode;
}) {
  return (
    <section
      id={`settings-${id}`}
      className="settings-section-panel w-full [&[hidden]]:hidden settings-stack flex flex-col gap-9"
      aria-labelledby={`settings-nav-${id}`}
      hidden={activeSection !== id}
    >
      {children}
    </section>
  );
}

export function SettingsGroup({
  title,
  description,
  action,
  children,
}: {
  title: string;
  description: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="settings-group flex flex-col gap-3 [&>header]:flex [&>header]:items-start [&>header]:justify-between [&>header]:gap-4 [&>header_h2]:text-heading [&>header_h2]:text-fg [&>header_p]:mt-0.5 [&>header_p]:text-small [&>header_p]:text-fg-muted">
      <header>
        <div>
          <h2>{title}</h2>
          <p>{description}</p>
        </div>
        {action && <div className="settings-group-action flex flex-none items-center">{action}</div>}
      </header>
      <div className="settings-group-body flex flex-col rounded-xl border border-line bg-surface shadow-card">
        {children}
      </div>
    </section>
  );
}

export function SettingRow({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-row grid min-h-16 grid-cols-[minmax(0,1fr)_minmax(180px,auto)] items-center gap-6 border-b border-line px-4 py-3 last:border-b-0 max-lg:grid-cols-1 max-lg:gap-3">
      <div className="settings-row-copy min-w-0 [overflow-wrap:anywhere] [&_h3]:text-small [&_h3]:font-semibold [&_h3]:text-fg [&_p]:mt-0.5 [&_p]:max-w-xl [&_p]:text-caption [&_p]:text-fg-muted">
        <h3>{title}</h3>
        <p>{description}</p>
      </div>
      <div className="settings-row-control flex min-w-0 flex-wrap items-center justify-end gap-2.5 max-lg:justify-start">
        {children}
      </div>
    </div>
  );
}

export function SettingChoice({
  title,
  description,
  options,
  labels = {},
  value,
  disabled,
  onChange,
}: {
  title: string;
  description: string;
  options: string[];
  labels?: Record<string, string>;
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <SettingRow title={title} description={description}>
      <div className="settings-segmented inline-flex flex-wrap justify-end gap-1 rounded-lg bg-surface-muted p-1 max-lg:justify-start">
        {options.map((option) => (
          <button
            key={option}
            type="button"
            className={
              value === option
                ? "settings-segment h-7 cursor-pointer rounded-md px-3 text-caption font-medium text-fg-muted transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-50 [&.active]:bg-surface [&.active]:text-brand [&.active]:shadow-raised active"
                : "settings-segment h-7 cursor-pointer rounded-md px-3 text-caption font-medium text-fg-muted transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-50 [&.active]:bg-surface [&.active]:text-brand [&.active]:shadow-raised"
            }
            disabled={disabled}
            onClick={() => onChange(option)}
          >
            {labels[option] ?? option.replace("_", " ")}
          </button>
        ))}
      </div>
    </SettingRow>
  );
}

export function TokenEditor({
  label,
  placeholder,
  values,
  draft,
  onDraft,
  disabled,
  onChange,
}: {
  label: string;
  placeholder: string;
  values: string[];
  draft: string;
  onDraft: (value: string) => void;
  disabled: boolean;
  onChange: (values: string[]) => void;
}) {
  const add = () => {
    const value = draft.trim().toLowerCase();
    if (!value) return;
    onDraft("");
    if (!values.includes(value)) onChange([...values, value]);
  };
  return (
    <div className="settings-token-editor flex flex-col gap-2.5 p-4">
      <div className="settings-token-label text-small font-semibold text-fg">{label}</div>
      <div className="settings-token-input-row flex gap-2">
        <input
          className="settings-text-input h-9 min-w-0 flex-1 rounded-md border border-line bg-surface px-3 text-small text-fg shadow-card outline-none focus:border-brand focus:ring-3 focus:ring-brand/15"
          aria-label={label}
          value={draft}
          placeholder={placeholder}
          disabled={disabled}
          onChange={(event) => onDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") add();
          }}
        />
        <DSButton disabled={disabled || !draft.trim()} onClick={add}>
          Add
        </DSButton>
      </div>
      <div className="settings-token-list flex flex-wrap gap-2">
        {values.length === 0 ? (
          <span className="settings-empty-inline inline-flex rounded-full bg-surface-muted px-2.5 py-1 text-caption text-fg-subtle">
            None
          </span>
        ) : (
          values.map((value) => (
            <button
              key={value}
              type="button"
              className="settings-token inline-flex h-6 cursor-pointer items-center gap-1 rounded-full bg-brand-soft px-2.5 text-caption font-medium text-brand"
              disabled={disabled}
              onClick={() => onChange(values.filter((item) => item !== value))}
            >
              {value} ×
            </button>
          ))
        )}
      </div>
    </div>
  );
}

export function IntegrationRow({
  title,
  detail,
  connected,
  action,
}: {
  title: string;
  detail: string;
  connected: boolean;
  action?: ReactNode;
}) {
  return (
    <div className="settings-row grid min-h-16 grid-cols-[minmax(0,1fr)_minmax(180px,auto)] items-center gap-6 border-b border-line px-4 py-3 last:border-b-0 max-lg:grid-cols-1 max-lg:gap-3">
      <div className="settings-row-copy min-w-0 [overflow-wrap:anywhere] [&_h3]:text-small [&_h3]:font-semibold [&_h3]:text-fg [&_p]:mt-0.5 [&_p]:max-w-xl [&_p]:text-caption [&_p]:text-fg-muted">
        <h3>{title}</h3>
        <p>{detail}</p>
      </div>
      <div className="settings-row-control flex min-w-0 flex-wrap items-center justify-end gap-2.5 max-lg:justify-start">
        <span
          className={`${connected ? "review-status-pill inline-flex h-5.5 min-w-16 items-center justify-center gap-1 rounded-full bg-slate-100 px-2 text-caption font-medium whitespace-nowrap text-slate-700 capitalize ring-1 ring-slate-500/15 ring-inset [&.lane-approved]:bg-emerald-50 [&.lane-approved]:text-emerald-700 [&.lane-approved]:ring-emerald-600/20 [&.lane-posted]:bg-emerald-50 [&.lane-posted]:text-emerald-700 [&.lane-posted]:ring-emerald-600/20 [&.lane-ready]:bg-emerald-50 [&.lane-ready]:text-emerald-700 [&.lane-ready]:ring-emerald-600/20 [&.lane-manual_required]:bg-amber-50 [&.lane-manual_required]:text-amber-800 [&.lane-manual_required]:ring-amber-600/25 [&.lane-stale]:bg-amber-50 [&.lane-stale]:text-amber-800 [&.lane-stale]:ring-amber-600/25 [&.lane-not_indexed]:bg-amber-50 [&.lane-not_indexed]:text-amber-800 [&.lane-not_indexed]:ring-amber-600/25 [&.lane-reviewing]:bg-indigo-50 [&.lane-reviewing]:text-indigo-700 [&.lane-reviewing]:ring-indigo-600/20 [&.lane-preview]:bg-indigo-50 [&.lane-preview]:text-indigo-700 [&.lane-preview]:ring-indigo-600/20 [&.lane-queued]:bg-sky-50 [&.lane-queued]:text-sky-700 [&.lane-queued]:ring-sky-600/20 [&.lane-indexing]:bg-sky-50 [&.lane-indexing]:text-sky-700 [&.lane-indexing]:ring-sky-600/20 [&.lane-superseded]:bg-sky-50 [&.lane-superseded]:text-sky-700 [&.lane-superseded]:ring-sky-600/20 [&.lane-failed]:bg-rose-50 [&.lane-failed]:text-rose-700 [&.lane-failed]:ring-rose-600/20 lane-approved" : "review-status-pill lane-failed"}`}
        >
          {connected ? "connected" : "not connected"}
        </span>
        {action}
      </div>
    </div>
  );
}
