<script setup lang="ts">
// server data: serialized into `<script id="__NUXT_DATA__">` (devalue)
const { data } = await useAsyncData("pricing", async () => ({ title: "Simple pricing for teams", plans: [{ id: "pro", plan: "Pro", cta: "Start for free today." }] }));
const n = ref(0);
const mounted = ref(false);
onMounted(() => (mounted.value = true));
</script>
<template>
  <main>
    <h1>{{ data?.title }}</h1>
    <p v-for="p in data?.plans" :key="p.id" :class="p.plan">{{ p.cta }}</p>
    <button @click="n++">Add to cart</button>
    <p>Cancel anytime</p>
    <p v-if="mounted">Ready when you are</p>
    <span :data-n="n"></span>
  </main>
</template>
