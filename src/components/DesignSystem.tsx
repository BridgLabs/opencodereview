import { Link, type LinkProps } from "@tanstack/react-router";
import { Check, ChevronDown, MoreHorizontal, MoreVertical } from "lucide-react";
import {
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type Ref,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

export const cx = (...classes: Array<string | false | null | undefined>) => classes.filter(Boolean).join(" ");

type ButtonTone = "primary" | "secondary" | "ghost" | "danger" | "link" | "dangerGhost";
type ButtonSize = "sm" | "md" | "icon" | "iconSm";
type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { tone?: ButtonTone; size?: ButtonSize };
type LinkButtonProps = LinkProps & { tone?: ButtonTone; size?: ButtonSize; children: ReactNode; className?: string };

const buttonToneClass: Record<ButtonTone, string> = {
  primary: "border-transparent bg-fg text-canvas hover:bg-white",
  secondary: "border-transparent bg-surface-muted text-fg hover:bg-surface-sunken",
  ghost: "border-transparent bg-transparent text-fg-muted hover:bg-surface-muted hover:text-fg",
  danger: "border-transparent bg-rose-100 text-rose-700 hover:bg-rose-200",
  link: "border-transparent bg-transparent text-brand-hover hover:bg-brand-soft",
  dangerGhost: "border-transparent bg-transparent text-danger hover:bg-rose-50",
};
const buttonSizeClass: Record<ButtonSize, string> = {
  sm: "h-7 px-2.5 text-caption",
  md: "h-8 px-3.5 text-small",
  icon: "size-8 text-small",
  iconSm: "size-7 text-caption",
};
const buttonBase =
  "inline-flex shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-lg border font-medium whitespace-nowrap transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand disabled:cursor-not-allowed disabled:opacity-50 [&_svg]:shrink-0";

export function DSButton({
  tone = "secondary",
  size = "md",
  className = "",
  ref,
  ...props
}: ButtonProps & { ref?: Ref<HTMLButtonElement> }) {
  return (
    <button
      ref={ref}
      type="button"
      className={cx(buttonBase, buttonSizeClass[size], buttonToneClass[tone], className)}
      {...props}
    />
  );
}

export function DSLinkButton({ tone = "secondary", size = "md", className = "", ...props }: LinkButtonProps) {
  return <Link className={cx(buttonBase, buttonSizeClass[size], buttonToneClass[tone], className)} {...props} />;
}

export type DSMenuItem = {
  label: string;
  icon?: ReactNode;
  hint?: string;
  tone?: "danger";
  disabled?: boolean;
  /** Set on every item to make the menu a single choice: items become radios and the chosen one is checked. */
  selected?: boolean;
  onSelect: () => void;
};

export function DSMenu({
  label,
  items,
  className,
  vertical = false,
  size = "icon",
  text,
  chevron = false,
}: {
  label: string;
  items: DSMenuItem[];
  className?: string;
  vertical?: boolean;
  size?: "icon" | "iconSm";
  text?: string;
  /** A select-style trigger: the text followed by a chevron instead of the "more" dots. */
  chevron?: boolean;
}) {
  const [position, setPosition] = useState<{ top: number; right: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!position) return;
    const close = (event: Event) => {
      if (
        event instanceof KeyboardEvent
          ? event.key === "Escape"
          : event.type !== "mousedown" ||
            ![button.current, menu.current].some((node) => node?.contains(event.target as Node))
      )
        setPosition(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    window.addEventListener("resize", close);
    document.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
      window.removeEventListener("resize", close);
      document.removeEventListener("scroll", close, true);
    };
  }, [position]);
  if (!items.length) return null;
  const toggle = () => {
    const rect = button.current?.getBoundingClientRect();
    setPosition(position || !rect ? null : { top: rect.bottom + 6, right: window.innerWidth - rect.right });
  };
  const Icon = chevron ? ChevronDown : vertical ? MoreVertical : MoreHorizontal;
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: only stops clicks bubbling to a clickable parent row; the button inside handles interaction
    // biome-ignore lint/a11y/useKeyWithClickEvents: same as above, there is no click action to mirror on the keyboard
    <div className={cx("relative inline-flex", className)} onClick={(event) => event.stopPropagation()}>
      <DSButton
        ref={button}
        size={text ? "md" : size}
        tone={vertical && !text ? "ghost" : "secondary"}
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={Boolean(position)}
        onClick={toggle}
      >
        {text}
        <Icon className="size-4" />
      </DSButton>
      {position &&
        createPortal(
          <div
            ref={menu}
            role="menu"
            aria-label={label}
            style={{ top: position.top, right: position.right }}
            className="fixed z-50 flex min-w-56 flex-col gap-0.5 rounded-2xl border border-line-strong bg-surface-muted p-1.5 shadow-pop"
          >
            {items.map((item) => (
              // biome-ignore lint/a11y/useAriaPropsSupportedByRole: aria-checked is only set on menuitemradio items, which support it
              <button
                key={item.label}
                type="button"
                role={item.selected === undefined ? "menuitem" : "menuitemradio"}
                aria-checked={item.selected}
                disabled={item.disabled}
                onClick={(event) => {
                  event.stopPropagation();
                  setPosition(null);
                  item.onSelect();
                }}
                className={cx(
                  "flex w-full cursor-pointer items-start gap-2 rounded-lg px-2.5 py-2 text-left text-small transition-colors hover:bg-surface-sunken disabled:cursor-not-allowed disabled:opacity-50 [&_svg]:mt-0.5 [&_svg]:size-3.5 [&_svg]:shrink-0",
                  item.tone === "danger" ? "text-danger" : "text-fg",
                )}
              >
                {item.selected === undefined ? (
                  item.icon
                ) : (
                  <Check className={item.selected ? "text-fg" : "invisible"} aria-hidden="true" />
                )}
                <span className="flex min-w-0 flex-col">
                  <span className="font-medium">{item.label}</span>
                  {item.hint && <span className="text-caption text-fg-subtle">{item.hint}</span>}
                </span>
              </button>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}

export type Tone = "neutral" | "brand" | "success" | "warning" | "danger" | "info" | "violet" | "teal";
const badgeTone: Record<Tone, string> = {
  neutral: "bg-surface-sunken text-fg-muted ring-transparent",
  brand: "bg-brand-soft text-brand-hover ring-brand/20",
  success: "bg-emerald-50 text-emerald-700 ring-emerald-600/20",
  warning: "bg-amber-50 text-amber-800 ring-amber-600/25",
  danger: "bg-rose-50 text-rose-700 ring-rose-600/20",
  info: "bg-sky-50 text-sky-700 ring-sky-600/20",
  violet: "bg-violet-50 text-violet-700 ring-violet-600/20",
  teal: "bg-teal-50 text-teal-700 ring-teal-600/20",
};
export const toneDot: Record<Tone, string> = {
  neutral: "bg-slate-400",
  brand: "bg-brand",
  success: "bg-emerald-500",
  warning: "bg-amber-500",
  danger: "bg-rose-500",
  info: "bg-sky-500",
  violet: "bg-violet-500",
  teal: "bg-teal-500",
};
export const toneText: Record<Tone, string> = {
  neutral: "text-fg-subtle",
  brand: "text-brand-hover",
  success: "text-emerald-600",
  warning: "text-amber-600",
  danger: "text-rose-600",
  info: "text-sky-600",
  violet: "text-violet-600",
  teal: "text-teal-600",
};

export function DSBadge({
  tone = "neutral",
  dot,
  icon,
  className = "",
  children,
  ...props
}: HTMLAttributes<HTMLSpanElement> & { tone?: Tone; dot?: boolean; icon?: ReactNode }) {
  return (
    <span
      className={cx(
        "inline-flex h-5.5 max-w-full items-center gap-1 rounded-md px-1.5 text-caption font-medium whitespace-nowrap ring-1 ring-inset [&>svg]:size-3 [&>svg]:shrink-0",
        badgeTone[tone],
        className,
      )}
      {...props}
    >
      {dot && <span className={cx("size-1.5 shrink-0 rounded-full", toneDot[tone])} />}
      {icon}
      <span className="min-w-0 truncate">{children}</span>
    </span>
  );
}

export function DSInput({
  className = "",
  icon,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & { icon?: ReactNode }) {
  const input = (
    <input
      className={cx(
        "h-9 w-full rounded-lg border border-transparent bg-surface-muted text-small text-fg outline-none placeholder:text-fg-subtle focus:border-line-strong focus:ring-3 focus:ring-brand/20 disabled:opacity-60",
        icon ? "pr-3 pl-9" : "px-3",
        className,
      )}
      {...props}
    />
  );
  if (!icon) return input;
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: the input is rendered inside the label through the `input` variable
    <label className="relative block">
      <span className="pointer-events-none absolute top-0 left-3 flex h-9 w-4 items-center justify-center text-fg-subtle [&>svg]:block [&>svg]:size-4">
        {icon}
      </span>
      {input}
    </label>
  );
}

export function DSEmptyState({ children }: { children: ReactNode }) {
  return <div className="rounded-xl bg-surface-muted/60 p-8 text-center text-small text-fg-muted">{children}</div>;
}

export function DSError({ children }: { children: ReactNode }) {
  return (
    <div
      role="alert"
      className="rounded-xl border border-rose-200 bg-rose-50 px-4 py-3 text-small font-medium text-rose-800"
    >
      {children}
    </div>
  );
}

export function DSSwitchIndicator({
  checked,
  onLabel = "On",
  offLabel = "Off",
}: {
  checked: boolean;
  onLabel?: string;
  offLabel?: string;
}) {
  return (
    <span className="inline-flex items-center gap-2 text-caption font-medium text-fg-muted">
      <span>{checked ? onLabel : offLabel}</span>
      <span
        className={cx(
          "relative h-5 w-9 rounded-full transition-colors",
          checked ? "bg-brand" : "bg-surface-sunken ring-1 ring-line-strong ring-inset",
        )}
      >
        <span
          className={cx(
            "absolute top-0.5 size-4 rounded-full bg-white shadow-raised transition-[left]",
            checked ? "left-4.5" : "left-0.5",
          )}
        />
      </span>
    </span>
  );
}
