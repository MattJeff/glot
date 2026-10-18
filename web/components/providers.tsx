"use client";

import { MutationCache, QueryCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "next-themes";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ApiError } from "@/lib/api";
import { installBridge } from "@/lib/preview-bridge";
import { useErrorMessage } from "@/lib/use-error-message";

function goToLogin() {
  const path = window.location.pathname;
  const prefix = path.match(/^\/(fr|es|de|ja|ar)(?=\/|$)/)?.[0] ?? "";
  if (!/\/(login|signup)$/.test(path)) {
    // Full navigation on purpose: drops all cached data from the expired session.
    // eslint-disable-next-line @next/next/no-location-assign-relative-destination
    window.location.assign(`${prefix}/login?next=${encodeURIComponent(path)}`);
  }
}

export function Providers({ children }: { children: React.ReactNode }) {
  const t = useTranslations("errors");
  const errorMessage = useErrorMessage();
  // No-op unless the dashboard is itself shown inside glot's locale matrix (dogfooding).
  useEffect(installBridge, []);
  const [client] = useState(() => {
    const onError = (err: Error) => {
      if (err instanceof ApiError && err.status === 401) {
        toast.error(t("sessionExpired"));
        goToLogin();
      }
    };
    return new QueryClient({
      queryCache: new QueryCache({ onError }),
      mutationCache: new MutationCache({
        onError: (err, _v, _c, mutation) => {
          if (err instanceof ApiError && err.status === 401 && !mutation.meta?.silent) onError(err);
          else if (!mutation.meta?.silent) toast.error(errorMessage(err));
        },
      }),
      defaultOptions: {
        queries: {
          staleTime: 15_000,
          retry: (n, err) => !(err instanceof ApiError && err.status >= 400 && err.status < 500) && n < 2,
        },
      },
    });
  });

  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
      <QueryClientProvider client={client}>
        <TooltipProvider delayDuration={200}>
          {children}
          <Toaster position="bottom-right" />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}
