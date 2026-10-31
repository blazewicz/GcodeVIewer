import "./style.css";
import { createControls, renderControls } from "./controls";
import { createRenderer, type LoadResult } from "./renderer";
import { colorByFeed, feedLegendVisible, hasFeed, stats, zLabel, zWindow } from "./state";
import { attachViewport, syncCanvasSize } from "./viewport";

function requireElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing element #${id}`);
  }
  return element as T;
}

const canvas = requireElement<HTMLCanvasElement>("view");
const panel = requireElement<HTMLElement>("panel");
const statsText = requireElement<HTMLParagraphElement>("stats");
const zSlider = requireElement<HTMLInputElement>("z");
const zLabelText = requireElement<HTMLElement>("zlabel");
const feedLegend = requireElement<HTMLElement>("feed-legend");
const feedMin = requireElement<HTMLInputElement>("feed-min");
const feedMax = requireElement<HTMLInputElement>("feed-max");
const fileInput = requireElement<HTMLInputElement>("file");

syncCanvasSize(canvas);

const api = await createRenderer(canvas).catch((error: unknown) => {
  statsText.textContent = `Renderer failed: ${String(error)}`;
  throw error;
});

const controls = createControls(api);
renderControls(requireElement("sliders"), controls.sliders);
renderControls(requireElement("toggles"), controls.toggles);
renderControls(requireElement("kinds"), controls.kinds);

stats.subscribe((text) => {
  statsText.textContent = text;
});

colorByFeed.subscribe((enabled) => {
  api.setColorByFeed(enabled);
  panel.classList.toggle("feed-colors", enabled);
});

feedLegendVisible.subscribe((visible) => {
  feedLegend.hidden = !visible;
});

zLabel.subscribe((text) => {
  zLabelText.textContent = text;
});

zWindow.subscribe((window) => {
  if (!window) {
    return;
  }
  zSlider.min = String(window.min);
  zSlider.max = String(window.max);
  zSlider.value = String(window.value);
  zSlider.disabled = window.max <= window.min;
  api.setZMax(window.value);
});

attachViewport(canvas, api, (message) => stats.set(message));

zSlider.addEventListener("input", () => {
  const current = zWindow.value;
  if (current) {
    zWindow.set({ ...current, value: Number(zSlider.value) });
  }
});

function applyFeedRange(): void {
  const min = Number(feedMin.value);
  const max = Number(feedMax.value);
  if (Number.isFinite(min) && Number.isFinite(max) && max > min) {
    api.setFeedRange(min, max);
  }
}

for (const input of [feedMin, feedMax]) {
  input.addEventListener("input", applyFeedRange);
}

function show(result: LoadResult): void {
  stats.set(`${result.moves.toLocaleString()} moves · ${result.segments.toLocaleString()} segments`);

  hasFeed.set(result.hasFeed);
  if (result.hasFeed) {
    // Round away the f32 representation noise so the inputs stay readable.
    feedMin.value = String(Math.round(result.feedMin * 100) / 100);
    feedMax.value = String(Math.round(result.feedMax * 100) / 100);
  }

  zWindow.set({ min: result.zMin, max: result.zMax, value: result.zMax });
}

/** Incremented per load so an older stream stops after a newer load starts. */
let loadGeneration = 0;

/**
 * Streams decoded text into the renderer in chunks. A newer load supersedes
 * one still in flight; the wasm loader is a single stateful session.
 */
async function loadStream(stream: ReadableStream<string>): Promise<void> {
  const generation = ++loadGeneration;
  const reader = stream.getReader();

  api.loadGcodeBegin();

  try {
    for (;;) {
      const { done, value } = await reader.read();

      if (generation !== loadGeneration) {
        await reader.cancel();
        return;
      }

      if (done) {
        break;
      }

      api.loadGcodeFeed(value);
    }

    show(api.loadGcodeEnd());
  } catch (error) {
    if (generation === loadGeneration) {
      stats.set(`Loading failed: ${String(error)}`);
    }
  }
}

async function loadFile(file: File): Promise<void> {
  await loadStream(file.stream().pipeThrough(new TextDecoderStream()));
}

fileInput.addEventListener("change", () => {
  const file = fileInput.files?.[0];
  if (file) {
    void loadFile(file);
  }
});

canvas.addEventListener("dragover", (event) => event.preventDefault());
canvas.addEventListener("drop", (event) => {
  event.preventDefault();
  const file = event.dataTransfer?.files?.[0];
  if (file) {
    void loadFile(file);
  }
});

requireElement<HTMLButtonElement>("demo").addEventListener("click", async () => {
  const response = await fetch("demo.gcode");
  if (response.body === null) {
    stats.set("Loading demo failed: response has no body");
    return;
  }
  await loadStream(response.body.pipeThrough(new TextDecoderStream()));
});

requireElement<HTMLButtonElement>("fit").addEventListener("click", () => api.fit());
