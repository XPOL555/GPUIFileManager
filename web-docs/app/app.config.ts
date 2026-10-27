const repo = 'https://github.com/XPOL555/GPUIFileManager'

export default defineAppConfig({
  seo: {
    title: 'FileManager',
    description: 'A lightweight Windows file manager in Rust that scales with many folders, tabs and windows open without growing into gigabytes of RAM.',
  },
  header: {
    title: 'FileManager',
  },
  repo: {
    url: repo,
    // The installers' file names carry the version: the releases page lists them. The bare exe has a stable name.
    releases: `${repo}/releases/latest`,
    exe: `${repo}/releases/latest/download/filemanager.exe`,
  },
  socials: {
    github: repo,
  },
  toc: {
    title: 'On this page',
    bottom: {
      title: 'FileManager',
      links: [
        { icon: 'i-lucide-download', label: 'Download', to: `${repo}/releases/latest`, target: '_blank' },
        { icon: 'i-simple-icons-github', label: 'Source', to: repo, target: '_blank' },
      ],
    },
  },
  ui: {
    colors: {
      primary: 'sky',
      neutral: 'zinc',
    },
  },
})
