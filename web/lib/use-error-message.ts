"use client";

import { useTranslations } from "next-intl";
import { ApiError } from "@/lib/api";

/** Server messages are shown as-is (English); transport errors are localized. */
export function useErrorMessage() {
  const t = useTranslations("errors");
  return (err: unknown) => {
    if (err instanceof ApiError) {
      if (err.status === 0) return t("network");
      if (err.status === 404) return t("notFound");
      return err.message || t("generic");
    }
    return t("generic");
  };
}
