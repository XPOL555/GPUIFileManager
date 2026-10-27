<script setup lang="ts">
// Logo icon + product name (Docus shows only the image when a logo is set).
const appConfig = useAppConfig()
</script>

<template>
  <span class="flex items-center gap-2">
    <img
      :src="appConfig.header?.logo?.light"
      :alt="appConfig.header?.logo?.alt"
      class="h-7 w-auto shrink-0 rounded-md"
    >
    <span class="font-semibold text-highlighted">{{ appConfig.header?.title }}</span>
  </span>
</template>
