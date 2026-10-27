<script setup lang="ts">
// Horizontal bars for benchmark rows: median as the bar, min–max as a whisker, one shared axis from 0.
// Highlighted rows (FileManager) in the primary color, the others in neutral gray. Used by BenchChart and BenchRows.
import type { BenchRow } from '~/utils/bench'

const props = defineProps<{
  title: string
  unit: string
  lowerIsBetter: boolean
  rows: BenchRow[]
  notes?: string
  // Labels are scenario names rather than app names: give them more room.
  wide?: boolean
}>()

// A little headroom so the whisker cap stays inside.
const top = computed(() => Math.max(...props.rows.map(r => r.max), 0) * 1.05 || 1)
const pct = (v: number) => `${(v / top.value) * 100}%`
const fmt = (v: number) => v >= 100 ? Math.round(v).toLocaleString('en-US') : v.toLocaleString('en-US', { maximumFractionDigits: 1 })
</script>

<template>
  <figure class="not-prose my-6 rounded-lg border border-default bg-default p-4">
    <figcaption class="mb-3 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
      <span class="font-semibold text-highlighted">{{ props.title }}</span>
      <span class="text-xs text-muted">
        {{ props.unit }}, median of runs · whisker: min–max ·
        <span class="font-medium">{{ props.lowerIsBetter ? 'lower is better' : 'higher is better' }}</span>
      </span>
    </figcaption>

    <div class="space-y-2">
      <div
        v-for="r in props.rows"
        :key="r.label"
        class="grid items-center gap-x-3 gap-y-1"
        :class="props.wide
          ? 'grid-cols-[1fr_5.5rem] sm:grid-cols-[14rem_1fr_6.5rem]'
          : 'grid-cols-[7.5rem_1fr_5.5rem] sm:grid-cols-[9rem_1fr_6.5rem]'"
        :title="`${r.label}: median ${fmt(r.median)} ${props.unit}, min ${fmt(r.min)}, max ${fmt(r.max)}, ${r.runs} runs`"
      >
        <span
          class="text-sm"
          :class="[
            r.highlight ? 'font-semibold text-highlighted' : 'text-toned',
            props.wide ? 'col-span-2 sm:col-span-1' : 'truncate',
          ]"
        >{{ r.label }}</span>
        <div class="relative h-5">
          <div
            class="absolute inset-y-0.5 left-0 rounded-r"
            :class="r.highlight ? 'bg-primary' : 'bg-neutral-400 dark:bg-neutral-500'"
            :style="{ width: pct(r.median) }"
          />
          <!-- min–max whisker with caps -->
          <div
            class="absolute top-1/2 h-px -translate-y-1/2 bg-neutral-900 dark:bg-neutral-100"
            :style="{ left: pct(r.min), width: `calc(${pct(r.max)} - ${pct(r.min)})` }"
          />
          <div
            class="absolute inset-y-1 w-px bg-neutral-900 dark:bg-neutral-100"
            :style="{ left: pct(r.min) }"
          />
          <div
            class="absolute inset-y-1 w-px bg-neutral-900 dark:bg-neutral-100"
            :style="{ left: pct(r.max) }"
          />
        </div>
        <span
          class="text-right text-sm tabular-nums"
          :class="r.highlight ? 'font-semibold text-highlighted' : 'text-toned'"
        >{{ fmt(r.median) }} {{ props.unit }}</span>
      </div>
    </div>

    <p
      v-if="props.notes"
      class="mt-3 text-xs text-muted"
    >
      {{ props.notes }}
    </p>

    <details class="mt-2 text-xs text-muted">
      <summary class="cursor-pointer select-none">
        Table
      </summary>
      <table class="mt-2 w-full tabular-nums">
        <thead>
          <tr class="text-left">
            <th class="py-1 font-medium">
              {{ props.wide ? 'Scenario' : 'App' }}
            </th>
            <th class="py-1 text-right font-medium">
              Median
            </th>
            <th class="py-1 text-right font-medium">
              Min
            </th>
            <th class="py-1 text-right font-medium">
              Max
            </th>
            <th class="py-1 text-right font-medium">
              Runs
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="r in props.rows"
            :key="r.label"
            class="border-t border-default"
          >
            <td class="py-1 pr-2">
              {{ r.label }}
            </td>
            <td class="py-1 text-right">
              {{ fmt(r.median) }} {{ props.unit }}
            </td>
            <td class="py-1 text-right">
              {{ fmt(r.min) }}
            </td>
            <td class="py-1 text-right">
              {{ fmt(r.max) }}
            </td>
            <td class="py-1 text-right">
              {{ r.runs }}
            </td>
          </tr>
        </tbody>
      </table>
    </details>
  </figure>
</template>
