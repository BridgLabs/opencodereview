import { createRootRoute, createRoute, createRouter, Navigate, Outlet } from "@tanstack/react-router";
import { ErrorScreen } from "./components/ErrorScreen";
import { Onboarding } from "./pages/Onboarding";
import { PlatformLayout } from "./platform/components/PlatformLayout";
import { IndexingPage } from "./platform/pages/IndexingPage";
import { ReviewsPage } from "./platform/pages/ReviewsPage";
import { SettingsPage } from "./platform/pages/SettingsPage";
import { SkillsPage } from "./platform/pages/SkillsPage";

const rootRoute = createRootRoute({ component: Outlet });

const onboardingRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: Onboarding });
const onboardingAliasRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/onboarding",
  component: Onboarding,
});
const platformRoute = createRoute({ getParentRoute: () => rootRoute, path: "/platform", component: PlatformLayout });
const platformIndexRoute = createRoute({
  getParentRoute: () => platformRoute,
  path: "/",
  component: () => <Navigate to="/platform/reviews" replace />,
});
const platformIndexingRoute = createRoute({
  getParentRoute: () => platformRoute,
  path: "/indexing",
  component: IndexingPage,
});
const platformReviewsRoute = createRoute({
  getParentRoute: () => platformRoute,
  path: "/reviews",
  component: ReviewsPage,
});
const platformSkillsRoute = createRoute({
  getParentRoute: () => platformRoute,
  path: "/skills",
  component: SkillsPage,
});
const platformSettingsRoute = createRoute({
  getParentRoute: () => platformRoute,
  path: "/settings",
  component: SettingsPage,
});

const routeTree = rootRoute.addChildren([
  onboardingRoute,
  onboardingAliasRoute,
  platformRoute.addChildren([
    platformIndexRoute,
    platformIndexingRoute,
    platformReviewsRoute,
    platformSkillsRoute,
    platformSettingsRoute,
  ]),
]);

export const router = createRouter({ routeTree, defaultErrorComponent: ErrorScreen });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
