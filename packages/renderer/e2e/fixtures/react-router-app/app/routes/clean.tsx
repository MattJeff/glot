import { useEffect, useState } from "react";
import { useLoaderData } from "react-router";
// every visible string comes from the loader (none hard-coded in the bundle) + one rendered after mount
export const loader = () => ({ title: "Simple pricing for teams", cta: "Start for free today.", button: "Add to cart", note: "Cancel anytime", plan: "Pro" });
export default function Clean() {
  const d = useLoaderData<typeof loader>();
  const [n, setN] = useState(0);
  const [ready, setReady] = useState("");
  useEffect(() => setReady("Ready when you are"), []);
  return (
    <main>
      <h1>{d.title}</h1>
      <p className={d.plan}>{d.cta}</p>
      <button onClick={() => setN(n + 1)}>{d.button}</button>
      <p>{d.note}</p>
      {ready && <p>{ready}</p>}
      <span data-n={n} />
    </main>
  );
}
