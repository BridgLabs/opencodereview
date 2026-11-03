import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { useQueryState } from "nuqs";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { RootLayout } from "./RootLayout";

function QueryValue() {
  const [tab] = useQueryState("tab");
  return <p>tab: {tab}</p>;
}

describe("RootLayout", () => {
  it("gives routed pages URL state from the router", async () => {
    const rootRoute = createRootRoute({ component: RootLayout });
    const pageRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: QueryValue });
    const router = createRouter({
      routeTree: rootRoute.addChildren([pageRoute]),
      history: createMemoryHistory({ initialEntries: ["/?tab=details"] }),
    });

    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => {
      root.render(<RouterProvider router={router} />);
      await router.load();
    });

    expect(container.textContent).toBe("tab: details");
    act(() => root.unmount());
  });
});
