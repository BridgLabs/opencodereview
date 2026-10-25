import { useState } from "react";
import type { ReviewCard } from "../../../types";

export function Avatar({ card }: { card: ReviewCard }) {
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const fallback = (card.author || "?").slice(0, 1).toUpperCase();
  return card.authorAvatarUrl && card.authorAvatarUrl !== failedUrl ? (
    <img
      className="size-7 rounded-full border border-line"
      src={card.authorAvatarUrl}
      alt={card.author}
      onError={() => setFailedUrl(card.authorAvatarUrl)}
    />
  ) : (
    <span className="grid size-7 place-items-center rounded-full border border-line bg-surface text-xs font-semibold text-fg-muted">
      {fallback}
    </span>
  );
}
