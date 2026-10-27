<script setup lang="ts">
// Landing hero in three.js: a long ribbon of file cards (the drive) flows through a glowing box (the listed
// folder). Cards keep their shape and place: crossing into the box they flash and settle on the accent color,
// leaving it they fade back to gray. FileManager reads the folder you are in and never walks the rest of the tree.
// Renders only while on screen and the tab is visible; with reduced motion it draws a single still frame.
import { onBeforeUnmount, onMounted, ref } from 'vue'

const el = ref<HTMLDivElement>()
// No WebGL (disabled GPU, some VMs): a still grid of tiles tells the same story.
const failed = ref(false)
const tiles = Array.from({ length: 12 * 8 }, (_, i) => {
  const x = i % 12, y = Math.floor(i / 12)
  return x >= 4 && x <= 7 && y >= 2 && y <= 5
})
let dispose = () => {}

// Page background as a THREE-parsable color: Tailwind 4 colors are oklch(), so let a canvas convert it.
function cssColorToHex(css: string): string {
  const c = document.createElement('canvas')
  c.width = c.height = 1
  const g = c.getContext('2d')!
  g.fillStyle = css
  g.fillRect(0, 0, 1, 1)
  const [r, gr, b] = g.getImageData(0, 0, 1, 1).data
  return `#${[r, gr, b].map(v => v!.toString(16).padStart(2, '0')).join('')}`
}

onMounted(async () => {
  const THREE = await import('three')
  const host = el.value
  if (!host) return

  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  let renderer: InstanceType<typeof THREE.WebGLRenderer>
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true })
  }
  catch {
    failed.value = true
    return
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
  host.appendChild(renderer.domElement)

  const scene = new THREE.Scene()
  const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 100)
  camera.position.set(0, 1.4, 11)
  camera.lookAt(0, 0, 0)

  scene.add(new THREE.HemisphereLight(0xffffff, 0x404050, 1.6))
  const sun = new THREE.DirectionalLight(0xffffff, 1.4)
  sun.position.set(4, 6, 8)
  scene.add(sun)

  const world = new THREE.Group()
  world.rotation.set(0.18, -0.55, 0)
  scene.add(world)

  // The ribbon: COLS cards along x (flowing), ROWS high, LANES deep.
  const COLS = 44, ROWS = 13, LANES = 5
  const SX = 0.62, SY = 0.42, SZ = 0.75
  const SPAN = COLS * SX
  const COUNT = COLS * ROWS * LANES
  const card = new THREE.BoxGeometry(0.5, 0.32, 0.025)
  const mat = new THREE.MeshStandardMaterial({ roughness: 0.45, metalness: 0.1 })
  const cards = new THREE.InstancedMesh(card, mat, COUNT)
  cards.instanceMatrix.setUsage(THREE.DynamicDrawUsage)
  world.add(cards)

  // Glow behind each card: a soft radial sprite, additive, black (invisible) unless the card flashes.
  const glowTex = (() => {
    const c = document.createElement('canvas')
    c.width = c.height = 64
    const g = c.getContext('2d')!
    const grad = g.createRadialGradient(32, 32, 0, 32, 32, 32)
    grad.addColorStop(0, 'rgba(255,255,255,1)')
    grad.addColorStop(0.4, 'rgba(255,255,255,0.35)')
    grad.addColorStop(1, 'rgba(255,255,255,0)')
    g.fillStyle = grad
    g.fillRect(0, 0, 64, 64)
    return new THREE.CanvasTexture(c)
  })()
  const glowGeo = new THREE.PlaneGeometry(1.1, 0.8)
  const glowMat = new THREE.MeshBasicMaterial({
    map: glowTex, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, toneMapped: false,
  })
  const glows = new THREE.InstancedMesh(glowGeo, glowMat, COUNT)
  glows.instanceMatrix.setUsage(THREE.DynamicDrawUsage)
  world.add(glows)

  // The listed folder.
  const BX = 2.4, BY = 1.6, BZ = (LANES * SZ) / 2 + 0.3
  const boxGeo = new THREE.BoxGeometry(BX * 2, BY * 2, BZ * 2)
  const edges = new THREE.LineSegments(
    new THREE.EdgesGeometry(boxGeo),
    new THREE.LineBasicMaterial({ color: 0x38bdf8, transparent: true, opacity: 0.95 }),
  )
  const glass = new THREE.Mesh(boxGeo, new THREE.MeshBasicMaterial({
    color: 0x38bdf8, transparent: true, opacity: 0.05, depthWrite: false, side: THREE.DoubleSide,
  }))
  world.add(edges, glass)

  const hot = new THREE.Color(0x38bdf8)
  const hotHi = new THREE.Color(0x7dd3fc)
  const cold = new THREE.Color()
  const tmp = new THREE.Color()
  const glow = new THREE.Color()
  const black = new THREE.Color(0, 0, 0)
  const m = new THREE.Matrix4()
  const q = new THREE.Quaternion()
  const pos = new THREE.Vector3()
  const scl = new THREE.Vector3()
  const Y_AXIS = new THREE.Vector3(0, 1, 0)
  // A stable per-card variation (some cards "selected" brighter, some files shorter). It must follow the card,
  // not the instance: instances shift back one column every time the ribbon has moved by SX.
  const hash = (n: number) => ((Math.sin(n * 12.9898) * 43758.5453) % 1 + 1) % 1

  function applyTheme() {
    const dark = document.documentElement.classList.contains('dark')
    cold.set(dark ? 0x3f3f46 : 0xe4e4e7)
    // Additive light only reads on a dark page; on a light one the card's own flash carries it.
    glows.visible = dark
    ;(edges.material as THREE.LineBasicMaterial).color.set(dark ? 0x38bdf8 : 0x0284c7)
    const bg = cssColorToHex(getComputedStyle(document.body).backgroundColor)
    scene.fog = new THREE.Fog(bg, 9, 22)
  }
  applyTheme()
  const themeObserver = new MutationObserver(applyTheme)
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })

  const smooth = (a: number, b: number, x: number) => {
    const t = Math.min(1, Math.max(0, (x - a) / (b - a)))
    return t * t * (3 - 2 * t)
  }

  function update(t: number) {
    const travel = t * 0.55
    const shift = travel % SX
    const passed = Math.floor(travel / SX)
    let i = 0
    for (let c = 0; c < COLS; c++) {
      const id = c - passed
      for (let r = 0; r < ROWS; r++) {
        for (let l = 0; l < LANES; l++, i++) {
          const x = c * SX - SPAN / 2 + shift
          const y0 = (r - (ROWS - 1) / 2) * SY
          const y = y0 + Math.sin(x * 0.35 + t * 0.6 + l) * 0.08
          const z = (l - (LANES - 1) / 2) * SZ + Math.sin(x * 0.25 - t * 0.4 + r * 0.3) * 0.18
          const s = hash(id * 97 + r * 7 + l)
          // Cards appear and disappear at the far ends of the ribbon by shrinking, never by popping.
          const ends = smooth(SPAN / 2, SPAN / 2 - 3, Math.abs(x))
          // Rows belong to the box or not by where they are, never by their wobble (no flicker at its top and
          // bottom). Along x: a short fade at each wall, and a flash right after the entry wall that dies off fast.
          const row = Math.abs(y0) < BY ? 1 : 0
          const inside = row * smooth(BX + 0.12, BX - 0.12, Math.abs(x))
          const d = x + BX
          const flash = row * (d < 0 ? Math.exp(d / 0.12) : Math.exp(-d / 0.45))
          pos.set(x, y, z)
          q.setFromAxisAngle(Y_AXIS, 0.3 * Math.sin(x * 0.2 + s * 6))
          scl.set(0.8 * (0.75 + 0.25 * s), 0.8, 0.8).multiplyScalar(ends)
          m.compose(pos, q, scl)
          cards.setMatrixAt(i, m)
          // Above 1 the color clips toward white: the card itself lights up during the flash.
          tmp.copy(cold).lerp(s > 0.93 ? hotHi : hot, inside).multiplyScalar(1 + 1.6 * flash)
          cards.setColorAt(i, tmp)
          pos.z -= 0.03
          m.compose(pos, q, scl)
          glows.setMatrixAt(i, m)
          glows.setColorAt(i, flash > 0.01 ? glow.copy(hot).multiplyScalar(flash * 0.9) : black)
        }
      }
    }
    cards.instanceMatrix.needsUpdate = true
    cards.instanceColor!.needsUpdate = true
    glows.instanceMatrix.needsUpdate = true
    glows.instanceColor!.needsUpdate = true
    ;(edges.material as THREE.LineBasicMaterial).opacity = 0.75 + 0.2 * Math.sin(t * 1.6)
  }
  // Gentle parallax toward the pointer.
  let px = 0, py = 0
  const onPointer = (e: PointerEvent) => {
    px = (e.clientX / window.innerWidth) * 2 - 1
    py = (e.clientY / window.innerHeight) * 2 - 1
  }
  window.addEventListener('pointermove', onPointer, { passive: true })

  function resize() {
    const w = host!.clientWidth, h = host!.clientHeight
    if (!w || !h) return
    renderer.setSize(w, h, false)
    camera.aspect = w / h
    // Keep the whole box in frame on narrow screens.
    camera.position.z = w / h < 1 ? 15 : 11
    camera.updateProjectionMatrix()
    if (reduced) frame(4)
  }
  const ro = new ResizeObserver(resize)
  ro.observe(host)

  function frame(t: number) {
    world.rotation.y += (-0.55 + px * 0.12 - world.rotation.y) * 0.05
    world.rotation.x += (0.18 + py * 0.06 - world.rotation.x) * 0.05
    update(t)
    renderer.render(scene, camera)
  }

  // Scene time advances by the frame's real duration, capped: after a paused tab or a long frame it picks up
  // where it stopped instead of jumping ahead.
  let raf = 0, visible = true, simT = 0, last = 0
  const loop = (now: number) => {
    raf = 0
    if (!visible || document.hidden) { last = 0; return }
    if (last) simT += Math.min(0.05, (now - last) / 1000)
    last = now
    frame(simT)
    raf = requestAnimationFrame(loop)
  }
  const start = () => { if (!reduced && !raf) raf = requestAnimationFrame(loop) }
  const io = new IntersectionObserver(([e]) => { visible = !!e?.isIntersecting; start() })
  io.observe(host)
  const onVisibility = () => start()
  document.addEventListener('visibilitychange', onVisibility)

  resize()
  if (reduced) frame(4)
  else start()

  dispose = () => {
    cancelAnimationFrame(raf)
    io.disconnect()
    ro.disconnect()
    themeObserver.disconnect()
    window.removeEventListener('pointermove', onPointer)
    document.removeEventListener('visibilitychange', onVisibility)
    card.dispose(); mat.dispose(); boxGeo.dispose()
    edges.geometry.dispose(); (edges.material as THREE.Material).dispose(); (glass.material as THREE.Material).dispose()
    cards.dispose()
    glows.dispose(); glowGeo.dispose(); glowMat.dispose(); glowTex.dispose()
    renderer.dispose()
    renderer.domElement.remove()
  }
})

onBeforeUnmount(() => dispose())
</script>

<template>
  <div class="relative">
    <div
      v-if="failed"
      class="grid aspect-[5/4] w-full grid-cols-12 content-center gap-1.5 p-6"
      aria-hidden="true"
    >
      <span
        v-for="(hot, i) in tiles"
        :key="i"
        class="h-5 rounded-sm"
        :class="hot ? 'bg-sky-400' : 'bg-zinc-200 dark:bg-zinc-800'"
      />
    </div>
    <div
      v-else
      ref="el"
      class="aspect-[5/4] w-full [mask-image:radial-gradient(closest-side,black_55%,transparent)] [&>canvas]:block [&>canvas]:size-full"
      role="img"
      aria-label="A long ribbon of file cards flows through a glowing box: the cards inside it are lit, the rest of the ribbon stays gray"
    />
    <div class="pointer-events-none absolute bottom-2 left-2 flex flex-col gap-1 text-xs text-muted sm:flex-row sm:gap-4">
      <span class="flex items-center gap-1.5"><span class="size-2.5 rounded-sm bg-sky-400" />the folder you are in</span>
      <span class="flex items-center gap-1.5"><span class="size-2.5 rounded-sm bg-zinc-300 dark:bg-zinc-700" />the rest of the drive, never walked</span>
    </div>
  </div>
</template>
