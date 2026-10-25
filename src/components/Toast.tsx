import { CheckCircle2, XCircle } from "lucide-react";
import { useEffect, useState } from "react";

type Toast = { id: number; message: string; tone: "success" | "error" };

let nextId = 1;
let toasts: Toast[] = [];
const listeners = new Set<(toasts: Toast[]) => void>();

function publish(next: Toast[]) {
  toasts = next;
  for (const listener of listeners) listener(toasts);
}

/** Shows a short confirmation at the bottom of the screen; it fades after a few seconds. */
export function toast(message: string, tone: Toast["tone"] = "success") {
  const id = nextId++;
  publish([...toasts, { id, message, tone }].slice(-3));
  setTimeout(() => publish(toasts.filter((item) => item.id !== id)), 3500);
}

/** Renders the toasts; mounted once at the app root. */
export function Toaster() {
  const [items, setItems] = useState(toasts);
  useEffect(() => {
    listeners.add(setItems);
    return () => {
      listeners.delete(setItems);
    };
  }, []);
  return (
    <div
      className="pointer-events-none fixed inset-x-0 bottom-[calc(1.25rem+env(safe-area-inset-bottom,0px))] z-[100] flex flex-col items-center gap-2 px-4"
      aria-live="polite"
    >
      {items.map((item) => (
        <div
          key={item.id}
          role="status"
          className="pointer-events-auto flex animate-[toast-in_180ms_ease-out] items-center gap-2 rounded-full border border-line-strong bg-surface-muted px-4 py-2 text-small font-medium text-fg shadow-pop"
        >
          {item.tone === "success" ? (
            <CheckCircle2 className="size-4 text-success" />
          ) : (
            <XCircle className="size-4 text-danger" />
          )}
          {item.message}
        </div>
      ))}
    </div>
  );
}
