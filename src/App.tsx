import { RouterProvider } from "@tanstack/react-router";
import { NuqsAdapter } from "nuqs/adapters/tanstack-router";
import { Toaster } from "./components/Toast";
import { router } from "./router";

export function App() {
  return (
    <NuqsAdapter>
      <RouterProvider router={router} />
      <Toaster />
    </NuqsAdapter>
  );
}
