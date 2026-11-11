import { useState } from "react";
// every string is hard-coded in the client bundle (no payload): only bundle localization keeps hydration clean
export function App() {
  const [n, setN] = useState(0);
  return (
    <main>
      <h1>Simple pricing for teams</h1>
      <p>Cancel anytime</p>
      <button onClick={() => setN(n + 1)}>Add to cart</button>
      <span data-plan={n ? "pro" : "free"} />
    </main>
  );
}
