import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { DSMenu } from "./DesignSystem";

afterEach(() => {
  vi.unstubAllGlobals();
});

function renderMenu() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const selected = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <DSMenu
        label="Account actions"
        items={["Sign in again", "Rename", "Remove account"].map((label) => ({ label, onSelect: selected }))}
      />,
    ),
  );
  return { container, root, selected };
}

it("keeps menu open for unrelated password-field scroll and allows item selection", async () => {
  const { container, root, selected } = renderMenu();
  const password = document.createElement("input");
  password.type = "password";
  document.body.append(password);
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Account actions"]')!.click());
    const menu = document.body.querySelector<HTMLElement>('[role="menu"]')!;
    await act(async () => password.dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
    await act(async () =>
      [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
        .find((item) => item.textContent?.includes("Sign in again"))!
        .click(),
    );
    expect(selected).toHaveBeenCalledOnce();
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    password.remove();
  }
});

it("closes menu on scroll from its trigger ancestor or document", async () => {
  const { container, root } = renderMenu();
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Account actions"]')!.click());
    await act(async () => container.dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(document.body.querySelector('[role="menu"]')).toBeNull();

    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Account actions"]')!.click());
    await act(async () => document.dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(document.body.querySelector('[role="menu"]')).toBeNull();

    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Account actions"]')!.click());
    await act(async () => window.dispatchEvent(new Event("scroll")));
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
