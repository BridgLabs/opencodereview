import { RouterProvider } from "@tanstack/react-router";
import { Toaster } from "./components/Toast";
import { router } from "./router";

export function App() {
  return (
    <>
      <RouterProvider router={router} />
      <Toaster />
    </>
  );
}
