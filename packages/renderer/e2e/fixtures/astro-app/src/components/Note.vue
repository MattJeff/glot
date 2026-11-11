<script setup>
import { onMounted, ref } from "vue";
defineProps({ label: String, kind: String });
const mounted = ref(false);
onMounted(() => (mounted.value = true));
</script>
<template>
  <section :class="kind"><p>{{ label }}</p><em>No credit card required</em><p v-if="mounted">Questions? Write to us</p></section>
</template>
