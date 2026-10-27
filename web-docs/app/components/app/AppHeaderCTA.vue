<script setup lang="ts">
// On the landing page, the way into the docs on the right of the header (in place of the search).
const route = useRoute()
// Marks the landing page so app.css can hide the header's mobile search button there.
useHead(() => ({ htmlAttrs: { 'data-landing': route.path === '/' ? '' : undefined } }))
</script>

<template>
  <UButton
    v-if="route.path === '/'"
    to="/getting-started/install"
    color="neutral"
    variant="ghost"
    trailing-icon="i-lucide-arrow-right"
    label="Docs"
  />
</template>
