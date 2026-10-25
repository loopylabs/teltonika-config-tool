// Small inline SVG icon set (24x24, stroke). No dependency; icons inherit the text colour.
const P = {
  devices: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
  overview: '<rect x="3" y="3" width="7" height="9" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="16" width="7" height="5" rx="1.5"/>',
  status: '<path d="M3 12h4l3-8 4 16 3-8h4"/>',
  settings: '<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>',
  list: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
  records: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  firmware: '<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4"/>',
  usb: '<circle cx="12" cy="19" r="2"/><path d="M12 17V4m0 0-2.5 2.5M12 4l2.5 2.5M12 12l-5-3v-2m5 5 5-2v-2"/><rect x="5.5" y="5" width="3" height="2"/><circle cx="17" cy="5.5" r="1.2"/>',
  bluetooth: '<path d="m7 7 10 10-5 5V2l5 5L7 17"/>',
  ble: '<path d="m7 7 10 10-5 5V2l5 5L7 17"/><path d="M3 9v6M21 9v6"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  refresh: '<path d="M20 11a8 8 0 0 0-14-4L4 9M4 4v5h5M4 13a8 8 0 0 0 14 4l2-2M20 20v-5h-5"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>',
  more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  power: '<path d="M12 3v9"/><path d="M6.3 6.3a8 8 0 1 0 11.4 0"/>',
  reset: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>',
  warn: '<path d="M12 3 2 20h20z"/><path d="M12 10v5M12 18h.01"/>',
  check: '<path d="m5 12 5 5 9-10"/>',
  plug: '<path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
} as const;
export type IconName = keyof typeof P;

export const icon = (name: IconName, size = 16): string =>
  `<svg class="ico" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${P[name]}</svg>`;
