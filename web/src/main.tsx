import { StrictMode, Suspense, lazy } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "./lib/queryClient";
import "./index.css";
import App from "./App.tsx";

// Devtools — only loaded in dev mode (tree-shaken from production builds)
const ReactQueryDevtools = lazy(() =>
  import("@tanstack/react-query-devtools").then((m) => ({
    default: m.ReactQueryDevtools,
  })),
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
      <Suspense fallback={null}>
        <ReactQueryDevtools buttonPosition="bottom-left" initialIsOpen={false} />
      </Suspense>
    </QueryClientProvider>
  </StrictMode>,
);
