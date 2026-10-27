<script setup lang="ts">
// One scenario of data/bench.json, one bar per app, best result first. `title` overrides the scenario's name;
// `hide-notes` leaves out its notes when the page already says them.
import bench from '~~/data/bench.json'
import { scenarioById } from '~/utils/bench'

const props = defineProps<{ id: string, title?: string, hideNotes?: boolean }>()

const scenario = computed(() => scenarioById(bench, props.id))
const rows = computed(() => {
  const s = scenario.value
  if (!s) return []
  return [...s.results]
    .sort((a, b) => s.lower_is_better ? a.median - b.median : b.median - a.median)
    .map(r => ({ ...r, label: r.app, highlight: r.app === 'FileManager' }))
})
</script>

<template>
  <BenchFigure
    v-if="scenario"
    :title="props.title || scenario.name"
    :unit="scenario.unit"
    :lower-is-better="scenario.lower_is_better"
    :rows="rows"
    :notes="props.hideNotes ? undefined : scenario.notes"
  />
  <UAlert
    v-else
    color="warning"
    variant="subtle"
    :title="`No scenario '${props.id}' in data/bench.json`"
  />
</template>
