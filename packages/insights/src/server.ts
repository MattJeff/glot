// Server-side variant (Next.js middleware / any Edge runtime): report a first visit without client JS.
export type ServerOptions = { key: string; endpoint: string; served: string };

/** Languages from Accept-Language, ordered by q. */
export function acceptLanguages(header: string | null): string[] {
  return (header || "")
    .split(",")
    .map((part) => {
      const [tag, ...params] = part.trim().split(";");
      const q = params.map((p) => p.trim()).find((p) => p.startsWith("q="));
      return { tag, q: q ? Number(q.slice(2)) : 1 };
    })
    .filter((x) => x.tag && x.tag !== "*" && x.q > 0)
    .sort((a, b) => b.q - a.q)
    .map((x) => x.tag)
    .slice(0, 5);
}

/** Posts one visit. Returns the experiment locale to serve when this visitor lands in the test arm. */
export async function reportVisit(req: Request, o: ServerOptions & { event?: string; arm?: string }) {
  const h = req.headers;
  const body = {
    langs: acceptLanguages(h.get("accept-language")),
    served: o.served,
    country: h.get("x-vercel-ip-country") || h.get("cf-ipcountry") || undefined,
    route: new URL(req.url).pathname,
    event: o.event,
    arm: o.arm,
  };
  try {
    const r = await fetch(`${o.endpoint}/api/insights/${o.key}`, { method: "POST", body: JSON.stringify(body) });
    return r.ok ? ((await r.json()) as { locale: string | null; arm: string | null }) : null;
  } catch {
    return null;
  }
}
