<script setup lang="ts">
// The `method` text of data/bench.json, as written by the benchmark harness.
import bench from '~~/data/bench.json'
</script>

<template>
  <p class="whitespace-pre-line">
    {{ bench.method }}
  </p>
</template>
