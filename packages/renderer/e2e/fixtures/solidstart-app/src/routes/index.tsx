import { createAsync, query } from "@solidjs/router";
import { createSignal, For, onMount, Show } from "solid-js";
// server data: serialized with seroval into `self.$R[…]` scripts, hydrated by `_$HY`
const pricing = query(async () => {
  "use server";
  return { title: "Simple pricing for teams", plans: [{ id: "pro", plan: "Pro", cta: "Start for free today." }] };
}, "pricing");
export const route = { preload: () => pricing() };
export default function Home() {
  const data = createAsync(() => pricing());
  const [n, setN] = createSignal(0);
  const [mounted, setMounted] = createSignal(false);
  onMount(() => setMounted(true));
  return (
    <main>
      <h1>{data()?.title}</h1>
      <For each={data()?.plans}>{(p) => <p class={p.plan}>{p.cta}</p>}</For>
      <button onClick={() => setN(n() + 1)}>Add to cart</button>
      <p>Cancel anytime</p>
      <Show when={mounted()}><p>Ready when you are</p></Show>
      <span data-n={n()} />
    </main>
  );
}
