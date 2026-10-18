import { getRequestConfig } from "next-intl/server";

// The app ships English only: every other browser language gets the on-device fallback.
export default getRequestConfig(async () => ({
  locale: "en",
  timeZone: "UTC",
  messages: (await import("../messages/en.json")).default,
}));
