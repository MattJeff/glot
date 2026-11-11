import { useEffect, useState } from "react";
import { useLoaderData } from "react-router";
// server data: streamed to the client with turbo-stream (`window.__reactRouterContext.streamController.enqueue(…)`)
export const loader = () => ({ title: "Simple pricing for teams", plans: [{ id: "pro", plan: "Pro", cta: "Start for free today." }] });
export default function Home() {
  const data = useLoaderData<typeof loader>();
  const [n, setN] = useState(0);
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  return (
    <main>
      <h1>{data.title}</h1>
      {data.plans.map((p) => <p key={p.id} className={p.plan}>{p.cta}</p>)}
      <button onClick={() => setN(n + 1)}>Add to cart</button>
      <p>Cancel anytime</p>
      {mounted && <p>Ready when you are</p>}
      <span data-n={n} />
    </main>
  );
}
