import { AlertTriangle } from "lucide-react";
import { cx, toneDot, toneText } from "../../../components/DesignSystem";
import type { ReviewCard } from "../../../types";
import { ArrowTabs } from "../../components/ArrowTabs";
import { Avatar } from "./Avatar";
import { itemTime, labelLane, laneTone, reviewDateGroups } from "./reviewHelpers";

type ReviewProcessColumn = { id: string; title: string; lanes: string[]; items: ReviewCard[] };

export function ReviewProcessColumns({
  compact = false,
  columns,
  activeIndex,
  selectedId,
  attentionPrIds,
  now,
  onActiveIndexChange,
  onSelect,
}: {
  /** Avatars only, for a list dragged narrow. */
  compact?: boolean;
  columns: ReviewProcessColumn[];
  activeIndex: number;
  selectedId: number | null;
  attentionPrIds: Set<number>;
  now: number;
  onActiveIndexChange: (index: number) => void;
  onSelect: (card: ReviewCard) => void;
}) {
  const activeColumn = columns[activeIndex];
  if (compact)
    return (
      <div className="review-process-compact flex flex-col items-center gap-1.5 pt-1">
        {columns
          .flatMap((column) => column.items)
          .map((card) => (
            <button
              key={card.id}
              type="button"
              aria-pressed={selectedId === card.id}
              aria-label={`${card.title} · ${labelLane(card.lane)}`}
              title={`${card.title}\n${card.repoFullName.split("/").pop()} #${card.number} · ${labelLane(card.lane)}`}
              className={cx(
                "relative grid size-12 cursor-pointer place-items-center rounded-xl transition-colors",
                selectedId === card.id ? "bg-surface-muted" : "hover:bg-surface-muted/50",
              )}
              onClick={() => onSelect(card)}
            >
              <Avatar card={card} />
              <span
                className={cx(
                  "absolute right-1.5 bottom-1.5 size-2 rounded-full ring-2 ring-surface",
                  toneDot[laneTone(card.lane)],
                )}
              />
            </button>
          ))}
      </div>
    );
  return (
    <div className="review-process-columns-shell flex flex-col gap-3">
      <ArrowTabs
        tabs={columns.map((column) => ({
          id: column.id,
          label: column.title,
          count: column.items.length,
        }))}
        activeId={columns[activeIndex]?.id ?? ""}
        panelId="review-process-group-panel"
        label="Review process groups"
        previousLabel="Previous review group"
        nextLabel="Next review group"
        onChange={(id) =>
          onActiveIndexChange(
            Math.max(
              0,
              columns.findIndex((column) => column.id === id),
            ),
          )
        }
      />
      {activeColumn && (
        <section
          id="review-process-group-panel"
          role="tabpanel"
          // biome-ignore lint/a11y/noNoninteractiveTabindex: WAI-ARIA tabs pattern: the panel is focusable so keyboard users can reach its content after the tab list
          tabIndex={0}
          className="review-process-column review-process-column-active flex flex-col gap-3 outline-none"
          aria-labelledby={`review-process-group-panel-tab-${activeColumn.id}`}
        >
          <div className="review-process-column-items flex flex-col gap-4">
            {reviewDateGroups(activeColumn.items, now).map((group) => (
              <section
                key={group.id}
                className="review-process-date-group flex flex-col gap-1.5"
                aria-labelledby={`review-process-date-group-${activeColumn.id}-${group.id}`}
              >
                <header className="review-process-date-group-header flex items-center justify-between px-2.5 text-caption font-medium text-fg-subtle">
                  <h4 id={`review-process-date-group-${activeColumn.id}-${group.id}`}>{group.title}</h4>
                  <span className="tabular-nums">{group.items.length}</span>
                </header>
                <div className="review-process-date-group-items flex flex-col gap-1.5">
                  {group.items.map((card) => (
                    <button
                      key={card.id}
                      type="button"
                      aria-pressed={selectedId === card.id}
                      className={`review-process-item flex w-full cursor-pointer items-center gap-3 rounded-xl border border-transparent px-2.5 py-2.5 text-left transition-colors ${selectedId === card.id ? "review-process-item-selected bg-surface-muted" : "hover:bg-surface-muted/50"}`}
                      onClick={() => onSelect(card)}
                    >
                      <Avatar card={card} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline gap-2">
                          <div
                            className="review-process-item-pr-title min-w-0 flex-1 truncate text-small font-medium text-fg"
                            title={card.title}
                          >
                            {card.title}
                          </div>
                          <time
                            className="flex-none text-caption text-fg-subtle tabular-nums"
                            dateTime={
                              Number.isFinite(card.updatedAt) ? new Date(card.updatedAt).toISOString() : undefined
                            }
                          >
                            {itemTime(card.updatedAt, now)}
                          </time>
                        </div>
                        <div className="review-process-item-meta mt-0.5 flex items-center gap-2">
                          <span className="review-process-item-title min-w-0 flex-1 truncate text-caption text-fg-subtle">
                            {card.repoFullName.split("/").pop()} <span className="tabular-nums">#{card.number}</span>
                          </span>
                          {attentionPrIds.has(card.id) && (
                            <AlertTriangle className="size-3.5 flex-none text-warning" aria-label="Needs attention" />
                          )}
                          <span
                            className={cx(
                              "review-process-lane inline-flex flex-none items-center gap-1.5 text-caption",
                              toneText[laneTone(card.lane)],
                            )}
                          >
                            <span className={cx("size-1.5 rounded-full", toneDot[laneTone(card.lane)])} />
                            {labelLane(card.lane)}
                          </span>
                        </div>
                      </div>
                    </button>
                  ))}
                </div>
              </section>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
