# @glot/insights

Cookieless locale-demand and conversion beacon for glot Insights (< 2 kB): which languages your visitors
want, which ones convert, and language / copy experiments. The server only stores daily counters.

```sh
npm i @glot/insights
```

```ts
import { glot } from "@glot/insights";
await glot.init({ key: "pk_…", endpoint: "https://glot.example.com", onLocale: (l) => switchLocale(l) });
glot.track("signup");
```

Server side (no client JS): `import { reportVisit } from "@glot/insights/server"`.

MIT — part of [glot](https://github.com/MattJeff/glot).
