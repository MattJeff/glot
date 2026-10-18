// <script src=".../review.global.js" data-project="acme" data-endpoint="https://glot.example.com" async>
// Loads review.app.global.js (same folder) only for reviewers.
import { reviewToken } from "./index";

const el = document.currentScript as HTMLScriptElement | null;
if (el && reviewToken(el.dataset.token)) {
  const s = document.createElement("script");
  s.src = el.src.replace(/review\.global\.js(\?.*)?$/, "review.app.global.js$1");
  s.async = true;
  s.onload = () => (window as any).glotReviewApp.start({
    project: el.dataset.project!, endpoint: el.dataset.endpoint, locale: el.dataset.locale, appUrl: el.dataset.appUrl, lang: el.dataset.lang, dir: el.dataset.dir as any, appOrigin: el.dataset.appOrigin, token: reviewToken(el.dataset.token),
  });
  document.head.append(s);
}
