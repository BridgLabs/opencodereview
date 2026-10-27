import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useRef } from "react";
import { DSButton } from "../../components/DesignSystem";

interface ArrowTab {
  id: string;
  label: string;
  count?: number;
}

export function ArrowTabs({
  tabs,
  activeId,
  panelId,
  label,
  previousLabel,
  nextLabel,
  showArrows = true,
  onChange,
}: {
  tabs: ArrowTab[];
  activeId: string;
  panelId: string;
  label: string;
  previousLabel: string;
  nextLabel: string;
  showArrows?: boolean;
  onChange: (id: string) => void;
}) {
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const activeIndex = Math.max(
    0,
    tabs.findIndex((tab) => tab.id === activeId),
  );
  useEffect(() => {
    buttons.current.get(activeId)?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [activeId]);
  if (!tabs.length) return null;
  const select = (index: number, focus = false) => {
    const tab = tabs[index];
    if (!tab) return;
    onChange(tab.id);
    if (focus) buttons.current.get(tab.id)?.focus();
  };
  return (
    <div className="review-process-column-switcher flex min-w-0 items-center gap-1">
      {showArrows && (
        <DSButton
          tone="ghost"
          size="iconSm"
          className="review-process-column-arrow"
          aria-label={previousLabel}
          title={previousLabel}
          disabled={activeIndex === 0}
          onClick={() => select(activeIndex - 1)}
        >
          <ChevronLeft className="size-4" />
        </DSButton>
      )}
      <div
        className="review-process-column-tabs flex min-w-0 flex-1 gap-1 overflow-x-auto [scrollbar-width:none]"
        role="tablist"
        aria-label={label}
        aria-orientation="horizontal"
      >
        {tabs.map((tab, index) => (
          <button
            key={tab.id}
            ref={(button) => {
              if (button) buttons.current.set(tab.id, button);
              else buttons.current.delete(tab.id);
            }}
            id={`${panelId}-tab-${tab.id}`}
            type="button"
            role="tab"
            aria-selected={activeIndex === index}
            aria-controls={panelId}
            tabIndex={activeIndex === index ? 0 : -1}
            title={tab.label}
            className={`review-process-column-tab inline-flex h-7 flex-none cursor-pointer items-center gap-1.5 rounded-full px-2.5 text-caption font-medium whitespace-nowrap transition-colors ${activeIndex === index ? "is-active bg-surface-muted text-fg" : "text-fg-muted hover:bg-surface-muted hover:text-fg"}`}
            onClick={() => select(index)}
            onKeyDown={(event) => {
              const next =
                event.key === "ArrowRight"
                  ? (index + 1) % tabs.length
                  : event.key === "ArrowLeft"
                    ? (index - 1 + tabs.length) % tabs.length
                    : event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? tabs.length - 1
                        : null;
              if (next === null) return;
              event.preventDefault();
              event.stopPropagation();
              select(next, true);
            }}
          >
            <span>{tab.label}</span>
            {tab.count !== undefined && (
              <span
                className={`review-process-column-tab-count rounded-full px-1.5 tabular-nums ${activeIndex === index ? "bg-surface-sunken" : "bg-surface-sunken text-fg-subtle"}`}
              >
                {tab.count}
              </span>
            )}
          </button>
        ))}
      </div>
      {showArrows && (
        <DSButton
          tone="ghost"
          size="iconSm"
          className="review-process-column-arrow"
          aria-label={nextLabel}
          title={nextLabel}
          disabled={activeIndex >= tabs.length - 1}
          onClick={() => select(activeIndex + 1)}
        >
          <ChevronRight className="size-4" />
        </DSButton>
      )}
    </div>
  );
}
