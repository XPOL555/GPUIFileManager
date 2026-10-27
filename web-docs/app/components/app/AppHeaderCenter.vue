<script setup lang="ts">
// Docus header center (search) on the docs pages only: the landing page keeps its header clear.
const route = useRoute()
</script>

<template>
  <UContentSearchButton
    v-if="route.path !== '/'"
    :collapsed="false"
    class="w-full"
    variant="soft"
    :ui="{
      leadingIcon: 'size-4 mx-0.5',
    }"
  />
  <div v-else />
</template>
