# FileManager — documentation site

Public documentation of FileManager, in English: Nuxt 4 +
[Docus](https://docus.dev), managed with bun, generated as a static site by `../.github/workflows/docs.yml` and
published on GitHub Pages at <https://xpol555.github.io/GPUIFileManager/>.

```
bun install
bun run dev        # http://localhost:3000
bun run generate   # static site in .output/public
```

- Pages: `content/` (numbered files and folders = sidebar order), landing page `content/index.md`.
- **Benchmark data: `data/bench.json`**, the file `bench/results.json` written by the harness in `../bench/`
  (same format). It sits outside `content/` because Docus turns every file there into a page; the components in
  `app/components/content/Bench*.vue` import it at build time: `<BenchChart id>` draws one scenario (one bar per
  app), `<BenchRows ids>` several FileManager-only scenarios as the rows of one chart. The Performance page is hidden until the
  benchmark is run again: it waits in `drafts/performance.md` (outside `content/`, so not generated). Move it
  back to `content/2.performance.md` and restore its links (landing button and section, `app/app.config.ts`
  toc) to publish it. It places every scenario by id, and its prose quotes the numbers: a new run with the same ids only needs the file
  replaced and the prose checked; a new id needs a chart on the page.
- Screenshots: `public/images/<name>.webp`, shown through `<Screenshot name="..." alt="..." ready>`
  (`app/components/content/Screenshot.vue`) so they follow the base URL; without `ready` it draws a placeholder.
- Landing hero: `app/components/content/HeroScene.vue` (three.js, loaded only in the browser; renders while on
  screen, a still frame with reduced motion, a static tile grid without WebGL). The landing page has no search in
  the header and a Docs link instead (`app/components/app/AppHeaderCenter.vue`, `AppHeaderCTA.vue`, `app/app.css`).
- Fonts: Space Grotesk and JetBrains Mono, set in `app/app.css`; `@nuxt/fonts` downloads and self-hosts them at
  build time.
- Sub-path builds: set `NUXT_APP_BASE_URL=/GPUIFileManager/` (from PowerShell or CI: Git Bash rewrites `/x/` into a
  Windows path).
- GitHub Pages must be enabled once in the repository settings: Pages > Source: **GitHub Actions**.
