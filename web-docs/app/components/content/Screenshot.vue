<script setup lang="ts">
// Screenshot from public/images/<name>.webp, resolved against app.baseURL (GitHub Pages sub-path).
// Until the image is added (and `ready` set on the tag in Markdown), a placeholder box with the alt text.
const props = defineProps<{ name: string, alt: string, ready?: boolean }>()
const base = useRuntimeConfig().app.baseURL.replace(/\/?$/, '/')
</script>

<template>
  <img
    v-if="props.ready"
    :src="`${base}images/${props.name}.webp`"
    :alt="props.alt"
    class="w-full rounded-lg border border-default shadow-lg"
    loading="lazy"
  >
  <div
    v-else
    class="flex aspect-[16/9] w-full flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-accented bg-muted/40 p-6 text-center text-sm text-muted"
  >
    <UIcon
      name="i-lucide-image"
      class="size-8"
    />
    <span>Screenshot coming soon</span>
    <span class="text-xs text-dimmed">{{ props.alt }}</span>
  </div>
</template>
