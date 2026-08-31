import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "./lib/queryClient";

import "./i18n";
import App from "./App";
import { TooltipProvider } from "./components/ui/Tooltip";
import "./index.css";
import "./styles.css";
import "./styles/theme.css";
import "./components/ui/ui.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={300}>
        <App />
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>,
);
