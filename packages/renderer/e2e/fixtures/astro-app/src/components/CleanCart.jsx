import { useEffect, useState } from "react";
// every visible string comes from the island props (none hard-coded in the bundle) + one rendered after mount
export default function CleanCart({ title, plans, note }) {
  const [n, setN] = useState(0);
  const [ready, setReady] = useState("");
  useEffect(() => setReady("Ready when you are"), []);
  return (
    <div>
      <p>{title}</p>
      {plans.map((p) => <button key={p.id} className={p.plan} onClick={() => setN(n + 1)}>{p.cta}</button>)}
      <p>{note}</p>
      {ready && <p>{ready}</p>}
      <span data-n={n} />
    </div>
  );
}
