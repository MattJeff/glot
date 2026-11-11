<script>
  import { onMount } from "svelte";
  let { data } = $props();
  let n = $state(0);
  let mounted = $state(false);
  onMount(() => (mounted = true));
</script>
<main>
  <h1>{data.title}</h1>
  {#each data.plans as p (p.id)}<p class={p.plan}>{p.cta}</p>{/each}
  <button onclick={() => n++}>Add to cart</button>
  <p>Cancel anytime</p>
  {#if mounted}<p>Ready when you are</p>{/if}
  <span data-n={n}></span>
</main>
