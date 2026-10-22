<script lang="ts" module>
  /** Stroke icons on a 24 px grid, drawn for this app (no icon font, no dependency). */
  const paths = {
    servers: ['M5 4h14a2 2 0 012 2v3a2 2 0 01-2 2H5a2 2 0 01-2-2V6a2 2 0 012-2z', 'M5 13h14a2 2 0 012 2v3a2 2 0 01-2 2H5a2 2 0 01-2-2v-3a2 2 0 012-2z', 'M7 7.5h.01M7 16.5h.01'],
    account: ['M12 12a4 4 0 100-8 4 4 0 000 8z', 'M4 21c0-3.9 3.6-6 8-6s8 2.1 8 6'],
    install: ['M12 3v12', 'M7.5 10.5L12 15l4.5-4.5', 'M4 17v2a2 2 0 002 2h12a2 2 0 002-2v-2'],
    host: ['M5 4h14a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2V6a2 2 0 012-2z', 'M8 21h8M12 17v4'],
    mods: ['M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3z', 'M4 7.5l8 4.5 8-4.5M12 12v9'],
    settings: ['M4 6h8M16 6h4M4 12h3M11 12h9M4 18h10M18 18h2', 'M14 6m-2 0a2 2 0 104 0 2 2 0 10-4 0', 'M9 12m-2 0a2 2 0 104 0 2 2 0 10-4 0', 'M16 18m-2 0a2 2 0 104 0 2 2 0 10-4 0'],
    info: ['M12 21a9 9 0 100-18 9 9 0 000 18z', 'M12 11v5M12 8h.01'],
    play: ['M8 5.5v13l11-6.5-11-6.5z'],
    minimize: ['M6 12h12'],
    maximize: ['M6.5 5h11A1.5 1.5 0 0119 6.5v11a1.5 1.5 0 01-1.5 1.5h-11A1.5 1.5 0 015 17.5v-11A1.5 1.5 0 016.5 5z'],
    restore: ['M9 8V6.5A1.5 1.5 0 0110.5 5h7A1.5 1.5 0 0119 6.5v7a1.5 1.5 0 01-1.5 1.5H16', 'M6.5 8h7A1.5 1.5 0 0115 9.5v8a1.5 1.5 0 01-1.5 1.5h-7A1.5 1.5 0 015 17.5v-8A1.5 1.5 0 016.5 8z'],
    close: ['M6 6l12 12M18 6L6 18'],
    chevron: ['M6 9l6 6 6-6'],
    check: ['M5 12.5l4.5 4.5L19 7'],
    alert: ['M12 3.5l9.5 16.5h-19L12 3.5z', 'M12 10v4.5M12 17.5h.01'],
    shield: ['M12 3l8 3v6c0 4.8-3.3 8-8 9-4.7-1-8-4.2-8-9V6l8-3z', 'M9 12l2.2 2.2L15.5 10'],
    eye: ['M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z', 'M12 15a3 3 0 100-6 3 3 0 000 6z'],
    refresh: ['M20 11a8 8 0 10-2.4 5.7', 'M20 4v7h-7'],
    folder: ['M3 7a2 2 0 012-2h4l2 2.5h8a2 2 0 012 2V17a2 2 0 01-2 2H5a2 2 0 01-2-2V7z'],
    external: ['M14 4h6v6M20 4l-9 9', 'M18 14v4a2 2 0 01-2 2H6a2 2 0 01-2-2V8a2 2 0 012-2h4'],
    logout: ['M9 4H6a2 2 0 00-2 2v12a2 2 0 002 2h3', 'M16 8l4 4-4 4M20 12H9'],
    trash: ['M4 7h16M10 11v6M14 11v6', 'M6 7l1 12a2 2 0 002 2h6a2 2 0 002-2l1-12', 'M9 7V5a1 1 0 011-1h4a1 1 0 011 1v2'],
    plus: ['M12 5v14M5 12h14'],
    edit: ['M4 20h4L19 9l-4-4L4 16v4z', 'M13.5 6.5l4 4'],
    globe: ['M12 21a9 9 0 100-18 9 9 0 000 18z', 'M3 12h18', 'M12 3c2.6 2.6 3.8 5.7 3.8 9s-1.2 6.4-3.8 9c-2.6-2.6-3.8-5.7-3.8-9S9.4 5.6 12 3z'],
    sun: ['M12 16a4 4 0 100-8 4 4 0 000 8z', 'M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6L7 7M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4'],
    moon: ['M20 14.5A8 8 0 019.5 4 8 8 0 1020 14.5z'],
    up: ['M6 15l6-6 6 6'],
    down: ['M6 9l6 6 6-6'],
    download: ['M12 4v11', 'M7.5 10.5L12 15l4.5-4.5', 'M5 20h14'],
    key: ['M14.5 9.5a4.5 4.5 0 11-9 0 4.5 4.5 0 019 0z', 'M13 13l7 7M17 17l2-2M15 19l2-2'],
    portal: ['M6 20V11a6 6 0 0112 0v9', 'M9.5 20v-8.5a2.5 2.5 0 015 0V20', 'M4 20h16'],
    box: ['M4 8l8-4 8 4v8l-8 4-8-4V8z', 'M4 8l8 4 8-4M12 12v8'],
  } as const;
  export type IconName = keyof typeof paths;
</script>

<script lang="ts">
  let { name, size = 18, filled = false, class: cls = '' }: { name: IconName; size?: number; filled?: boolean; class?: string } = $props();
</script>

<svg
  class="icon {cls}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? 'currentColor' : 'none'}
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each paths[name] as d (d)}<path {d} />{/each}
</svg>

<style>
  .icon { flex: none; display: block; }
</style>
