# @glot/feedback

Let native speakers flag a wrong, awkward or unclear translation in production (< 4 kB, Shadow DOM,
accessible). Select text → "Improve translation". No cookie, no storage, no visitor id.

```sh
npm i @glot/feedback
```

```ts
import { glotFeedback } from "@glot/feedback";
glotFeedback.init({ key: "pk_…", endpoint: "https://glot.example.com", labels: { report: "Améliorer la traduction" } });
```

Or without a bundler: `<script src="https://unpkg.com/@glot/feedback/dist/feedback.global.js"></script>`.
Reports land in the glot server dashboard (Feedback).

MIT — part of [glot](https://github.com/MattJeff/glot).
