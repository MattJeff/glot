import type { PlansInfo } from "@/lib/api";

/** Plans and Stripe prices, fetched on the server (5 min cache) so the HTML carries real prices. */
export async function getPlans(): Promise<PlansInfo | undefined> {
  try {
    const r = await fetch(`${process.env.GLOT_API_URL ?? "http://localhost:8790"}/api/billing/plans`, { next: { revalidate: 300 } });
    return r.ok ? ((await r.json()) as PlansInfo) : undefined;
  } catch {
    return undefined; // API down: the client query fills prices in
  }
}
