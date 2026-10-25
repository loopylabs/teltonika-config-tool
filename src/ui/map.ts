// Position map: Leaflet with OpenStreetMap tiles, loaded only when the Telemetry page is opened.
// The map element lives outside the re-rendered status body and is moved into its slot.
import type * as Leaflet from "leaflet";

const TRAIL_CAP = 2000;

type MapState = {
  L: typeof Leaflet;
  map: Leaflet.Map;
  marker: Leaflet.CircleMarker;
  trail: Leaflet.Polyline;
  tileErrors: number;
  tileOk: boolean;
};

let el: HTMLDivElement | null = null;
let state: MapState | null = null;
let loading: Promise<void> | null = null;
let trail: [number, number][] = [];
let onFallback: (offline: boolean) => void = () => {};

export function mapElement(): HTMLDivElement {
  if (!el) {
    el = document.createElement("div");
    el.className = "mapbox";
    el.setAttribute("role", "region");
    el.setAttribute("aria-label", "Map of the current position");
  }
  return el;
}

/** Offline when the browser says so or tiles keep failing without one loading. */
export const isOffline = () => !navigator.onLine || (state !== null && state.tileErrors >= 4 && !state.tileOk);

async function ensure(): Promise<void> {
  if (state) return;
  loading ??= (async () => {
    const [L] = await Promise.all([import("leaflet"), import("leaflet/dist/leaflet.css")]);
    const map = L.map(mapElement(), { zoomControl: true, attributionControl: true }).setView([0, 0], 2);
    const layer = L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", {
      maxZoom: 19,
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer">OpenStreetMap</a> contributors',
    });
    const s: MapState = {
      L,
      map,
      marker: L.circleMarker([0, 0], { radius: 7, weight: 2, color: "#fff", fillColor: "#2563eb", fillOpacity: 1 }),
      trail: L.polyline([], { color: "#2563eb", weight: 3, opacity: 0.7 }),
      tileErrors: 0,
      tileOk: false,
    };
    layer.on("tileerror", () => {
      s.tileErrors++;
      onFallback(isOffline());
    });
    layer.on("tileload", () => {
      s.tileOk = true;
      s.tileErrors = 0;
      onFallback(isOffline());
    });
    layer.addTo(map);
    s.trail.addTo(map);
    state = s;
  })();
  await loading;
}

/** Called after the status body is re-rendered: put the map into its slot. */
export function attachMap(slot: HTMLElement, onOfflineChange: (offline: boolean) => void) {
  onFallback = onOfflineChange;
  slot.appendChild(mapElement());
  state?.map.invalidateSize();
}

export function detachMap() {
  el?.remove();
}

/** Moves the marker and extends the breadcrumb trail. */
export async function showPosition(lat: number, lon: number): Promise<void> {
  const last = trail[trail.length - 1];
  if (!last || last[0] !== lat || last[1] !== lon) {
    trail.push([lat, lon]);
    if (trail.length > TRAIL_CAP) trail = trail.slice(-TRAIL_CAP);
  }
  try {
    await ensure();
  } catch {
    onFallback(true);
    return;
  }
  const s = state!;
  s.trail.setLatLngs(trail);
  s.marker.setLatLng([lat, lon]);
  if (!s.map.hasLayer(s.marker)) {
    s.marker.addTo(s.map);
    s.map.setView([lat, lon], 15);
  } else if (!s.map.getBounds().contains([lat, lon])) {
    s.map.panTo([lat, lon]);
  }
}

export const clearTrail = () => {
  trail = [];
  state?.trail.setLatLngs([]);
};
