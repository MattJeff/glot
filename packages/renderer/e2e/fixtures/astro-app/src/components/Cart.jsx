import { useEffect, useState } from "react";
// island props are serialized into `<astro-island props="…">`
export default function Cart({ title, plans }) {
  const [n, setN] = useState(0);
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  return (
    <div>
      <p>{title}</p>
      {plans.map((p) => <button key={p.id} className={p.plan} onClick={() => setN(n + 1)}>{p.cta}</button>)}
      <p>Cancel anytime</p>
      {mounted && <p>Ready when you are</p>}
      <span data-n={n} />
    </div>
  );
}
