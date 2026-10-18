// The one-line install: the loader is ~1 kB and does nothing unless the URL carries a review link
// (`?glot_review=glr_…`) or the tab already holds one. Normal visitors download nothing more.
// Served from public/glot → packages/review/dist (see `predev`); in production use unpkg or your CDN.
const ENDPOINT = process.env.NEXT_PUBLIC_GLOT_ENDPOINT ?? "http://localhost:8832";
const PROJECT = process.env.NEXT_PUBLIC_GLOT_PROJECT ?? "relay";
const APP = process.env.NEXT_PUBLIC_GLOT_APP_ORIGIN ?? "http://localhost:3520";

export function Review({ locale }: { locale: string }) {
  return <script src="/glot/review.global.js" data-project={PROJECT} data-endpoint={ENDPOINT} data-locale={locale} data-app-origin={APP} async />;
}
