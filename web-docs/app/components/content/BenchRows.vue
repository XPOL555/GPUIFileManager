<script setup lang="ts">
// Several FileManager-only scenarios of data/bench.json as the rows of one chart (same unit), in the given order.
// `ids` is space-separated; `strip` is a prefix removed from the scenario names to make the row labels.
import bench from '~~/data/bench.json'
import { scenarioById } from '~/utils/bench'

const props = defineProps<{ ids: string, title: string, strip?: string, notes?: string }>()

const scenarios = computed(() => props.ids.split(/\s+/).filter(Boolean).map(id => scenarioById(bench, id)))
const missing = computed(() => props.ids.split(/\s+/).filter(Boolean).filter((_, i) => !scenarios.value[i]))
const found = computed(() => scenarios.value.filter(s => !!s))
const rows = computed(() => found.value.map((s) => {
  const r = s.results.find(r => r.app === 'FileManager') ?? s.results[0]!
  const label = props.strip && s.name.startsWith(props.strip) ? s.name.slice(props.strip.length) : s.name
  return { ...r, label, highlight: true }
}))
// The scenarios' own note when they all share it.
const notes = computed(() => props.notes ?? (new Set(found.value.map(s => s.notes)).size === 1 ? found.value[0]?.notes : undefined))
</script>

<template>
  <BenchFigure
    v-if="found.length && !missing.length"
    :title="props.title"
    :unit="found[0]!.unit"
    :lower-is-better="found[0]!.lower_is_better"
    :rows="rows"
    :notes="notes"
    wide
  />
  <UAlert
    v-else
    color="warning"
    variant="subtle"
    :title="`Missing in data/bench.json: ${missing.join(', ')}`"
  />
</template>
