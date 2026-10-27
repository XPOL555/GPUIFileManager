// Static documentation site for FileManager.
// Built by .github/workflows/docs.yml with `bun run generate`; NUXT_APP_BASE_URL sets the sub-path on GitHub Pages
// (/GPUIFileManager/), so every static asset referenced outside Markdown goes through `base`.
const base = (process.env.NUXT_APP_BASE_URL || '/').replace(/\/?$/, '/')

export default defineNuxtConfig({
  extends: ['docus'],
  site: {
    name: 'FileManager',
  },
  app: {
    head: {
      link: [{ rel: 'icon', type: 'image/png', href: `${base}favicon.png` }],
    },
  },
  appConfig: {
    github: {
      owner: 'XPOL555',
      name: 'GPUIFileManager',
      url: 'https://github.com/XPOL555/GPUIFileManager',
      branch: 'main',
      rootDir: 'web-docs',
    },
    header: {
      logo: {
        light: `${base}images/logo.png`,
        dark: `${base}images/logo.png`,
        alt: 'FileManager',
      },
    },
  },
  // Images are served as they are. IPX is not prerendered under a sub-path and would double the base in its URLs.
  image: {
    provider: 'none',
  },
  // <page>/index.html, not <page>.html: next to it Nitro writes a <page>/ folder (_payload.json), and a static host
  // that redirects /page to that folder would 404 on every direct link without an index.html in it.
  nitro: {
    prerender: {
      autoSubfolderIndex: true,
    },
  },
  // robots.txt only works at the domain root: skip it when the site lives in a sub-path.
  robots: base === '/' ? {} : { robotsTxt: false },
  mcp: {
    enabled: false,
  },
  llms: {
    domain: process.env.NUXT_SITE_URL || 'http://localhost:3000',
    description: 'Documentation of FileManager, a lightweight Windows file manager in Rust that stays light with many folders open.',
  },
})
