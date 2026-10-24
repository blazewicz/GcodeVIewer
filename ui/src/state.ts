import { computed, signal } from "./signals";

export interface ZWindow {
  min: number;
  max: number;
  value: number;
}

export const stats = signal("Drop a G-code file onto the canvas.");
export const hasFeed = signal(false);
export const colorByFeed = signal(false);
export const zWindow = signal<ZWindow | null>(null);

export const feedLegendVisible = computed(
  () => colorByFeed.value && hasFeed.value,
  colorByFeed,
  hasFeed,
);

export const zLabel = computed(() => {
  const bounds = zWindow.value;
  return bounds === null ? "Z" : `Z ≤ ${bounds.value.toFixed(2)} mm`;
}, zWindow);
