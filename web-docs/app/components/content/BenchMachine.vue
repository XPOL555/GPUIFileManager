<script setup lang="ts">
// When the benchmark was run, on what, and the FileManager build measured. A commit ending in "-dirty" means
// that commit plus uncommitted work.
import bench from '~~/data/bench.json'

const repo = 'https://github.com/XPOL555/GPUIFileManager'
const [sha, dirty] = (() => {
  const c = bench.commit || ''
  return [c.replace(/-dirty$/, ''), c.endsWith('-dirty')]
})()
</script>

<template>
  <dl class="not-prose my-6 grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 rounded-lg border border-default p-4 text-sm">
    <dt class="text-muted">
      Date
    </dt>
    <dd>{{ bench.date }}</dd>
    <dt class="text-muted">
      FileManager
    </dt>
    <dd>
      {{ bench.app_version }}
      <template v-if="sha">
        · <a
          :href="`${repo}/commit/${sha}`"
          target="_blank"
          class="font-mono text-primary hover:underline"
        >{{ sha.slice(0, 7) }}</a>
        <span
          v-if="dirty"
          class="text-muted"
        > + work in progress</span>
      </template>
    </dd>
    <dt class="text-muted">
      CPU
    </dt>
    <dd>{{ bench.machine.cpu }}</dd>
    <dt class="text-muted">
      RAM
    </dt>
    <dd>{{ bench.machine.ram_gb }} GB</dd>
    <dt class="text-muted">
      Disk
    </dt>
    <dd>{{ bench.machine.disk }}</dd>
    <dt class="text-muted">
      OS
    </dt>
    <dd>{{ bench.machine.os }}</dd>
  </dl>
</template>
