export async function openExternal(url: string): Promise<void> {
  if (!url) return;
  window.open(url, "_blank", "noopener,noreferrer");
}
