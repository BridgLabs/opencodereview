import { type PointerEvent as ReactPointerEvent, useCallback, useEffect, useState } from "react";

/** Below this width while dragging, the column snaps to its compact width. */
const SNAP_BELOW = 200;

function stored(key: string, fallback: number) {
  try {
    const value = Number(localStorage.getItem(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  } catch {
    return fallback;
  }
}

/**
 * A column width the user drags from its right edge, remembered per browser. Dragging it narrow
 * snaps it to `compact`; double-clicking the handle restores `initial`.
 */
export function useResizableWidth(
  key: string,
  { initial, compact, max }: { initial: number; compact: number; max: number },
) {
  const [width, setWidth] = useState(() => Math.min(max, Math.max(compact, stored(key, initial))));
  const [dragging, setDragging] = useState(false);
  useEffect(() => {
    try {
      localStorage.setItem(key, String(width));
    } catch {
      // Storage is a convenience; the width still applies for this visit.
    }
  }, [key, width]);
  const onPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLElement>) => {
      event.preventDefault();
      const startX = event.clientX;
      const startWidth = width;
      setDragging(true);
      const move = (moveEvent: PointerEvent) => {
        const next = startWidth + moveEvent.clientX - startX;
        setWidth(next < SNAP_BELOW ? compact : Math.min(max, next));
      };
      const up = () => {
        setDragging(false);
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    },
    [width, compact, max],
  );
  return {
    width,
    compact: width <= compact,
    dragging,
    handleProps: {
      onPointerDown,
      onDoubleClick: () => setWidth(initial),
    },
  };
}
