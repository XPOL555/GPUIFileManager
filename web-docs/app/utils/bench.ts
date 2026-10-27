// Shape of data/bench.json: the file the harness in bench/ writes as bench/results.json.
export type BenchResult = { app: string, median: number, min: number, max: number, runs: number }
export type BenchScenario = {
  id: string
  name: string
  unit: string
  lower_is_better: boolean
  notes?: string
  results: BenchResult[]
}
export type Bench = { scenarios: BenchScenario[] }

export function scenarioById(bench: Bench, id: string): BenchScenario | undefined {
  return bench.scenarios.find(s => s.id === id)
}

// One bar of a chart: an app (BenchChart) or a scenario (BenchRows).
export type BenchRow = { label: string, median: number, min: number, max: number, runs: number, highlight?: boolean }
