import type { ReactNode } from "react";

function PlatformHeader({ title, description, action }: { title: string; description: string; action?: ReactNode }) {
  return (
    <header data-page-header className="flex flex-none flex-wrap items-center justify-between gap-4 px-6 pt-5 pb-4">
      <div className="min-w-0">
        <h1 className="text-title text-fg">{title}</h1>
        <p className="mt-0.5 max-w-3xl text-small text-fg-muted">{description}</p>
      </div>
      {action && <div className="flex shrink-0 items-center gap-2">{action}</div>}
    </header>
  );
}

export function PlatformPage({
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
    <section className="flex h-full min-h-0 flex-col">
      <PlatformHeader title={title} description={description} action={action} />
      {children}
    </section>
  );
}
