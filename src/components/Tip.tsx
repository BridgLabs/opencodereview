import { Info } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { createPortal } from "react-dom";
import { cx } from "./DesignSystem";

const EDGE = 12;
const MAX_WIDTH = 288;

/**
 * Shows `content` above its children right away on hover or keyboard focus. It renders in a portal,
 * so table cells and scroll containers that clip their overflow do not cut it off.
 */
export function Tip({ content, children, className }: { content: ReactNode; children: ReactNode; className?: string }) {
  const id = useId();
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const show = (target: Element) => setAnchor(target.getBoundingClientRect());
  const hide = () => setAnchor(null);
  const left = anchor
    ? Math.min(Math.max(anchor.left + anchor.width / 2, EDGE + MAX_WIDTH / 2), window.innerWidth - EDGE - MAX_WIDTH / 2)
    : 0;
  const below = anchor ? anchor.top < 80 : false;
  return (
    <>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: hover and focus bubble up from the child it describes; the wrapper itself is never interactive */}
      <span
        className={cx("inline-flex", className)}
        aria-describedby={anchor ? id : undefined}
        onMouseEnter={(event) => show(event.currentTarget)}
        onMouseLeave={hide}
        onFocus={(event) => show(event.currentTarget)}
        onBlur={hide}
      >
        {children}
      </span>
      {anchor &&
        createPortal(
          <span
            role="tooltip"
            id={id}
            className="pointer-events-none fixed z-50 w-max rounded-lg border border-line-strong bg-surface-muted px-2.5 py-1.5 text-left text-caption font-normal text-fg shadow-pop"
            style={{
              left,
              top: below ? anchor.bottom + 8 : anchor.top - 8,
              maxWidth: MAX_WIDTH,
              transform: below ? "translateX(-50%)" : "translate(-50%, -100%)",
            }}
          >
            {content}
          </span>,
          document.body,
        )}
    </>
  );
}

/** An info icon that explains the label next to it, by mouse or keyboard. */
export function Hint({ text, className }: { text: string; className?: string }) {
  return (
    <Tip content={text}>
      <button
        type="button"
        aria-label={text}
        className={cx(
          "inline-grid cursor-help place-items-center rounded-full text-fg-subtle hover:text-fg-muted focus-visible:outline-2 focus-visible:outline-brand",
          className,
        )}
      >
        <Info className="size-3.5" aria-hidden="true" />
      </button>
    </Tip>
  );
}
