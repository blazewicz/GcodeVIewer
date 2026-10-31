import init, { Renderer, type PathStats } from "../pkg/gcode_viewer_wasm.js";

export interface LoadResult {
  moves: number;
  segments: number;
  zMin: number;
  zMax: number;
  feedMin: number;
  feedMax: number;
  hasFeed: boolean;
}

export interface RendererApi {
  loadGcode(text: string): LoadResult;
  /** Starts a chunked load; feed it with `loadGcodeFeed` and finish with `loadGcodeEnd`. */
  loadGcodeBegin(): void;
  /** Feeds one chunk of text; the chunk may end mid-line. */
  loadGcodeFeed(text: string): void;
  /** Finishes the load and reports its summary. */
  loadGcodeEnd(): LoadResult;
  /** Whether the rendered image changed since the last `render` call. */
  needsRedraw(): boolean;
  render(): void;
  resize(): void;
  fit(): void;
  pan(dx: number, dy: number): void;
  zoom(factor: number, x: number, y: number): void;
  orbit(deltaYaw: number, deltaPitch: number): void;
  setLineWidth(width: number): void;
  setExtrusionDiameter(diameter: number): void;
  setZMax(z: number): void;
  setKindVisible(kind: number, visible: boolean): void;
  setColorByFeed(byFeed: boolean): void;
  setFeedRange(min: number, max: number): void;
}

function toLoadResult(stats: PathStats): LoadResult {
  return {
    moves: stats.moves,
    segments: stats.segments,
    zMin: stats.z_min,
    zMax: stats.z_max,
    feedMin: stats.feed_min,
    feedMax: stats.feed_max,
    hasFeed: stats.has_feed,
  };
}

export async function createRenderer(canvas: HTMLCanvasElement): Promise<RendererApi> {
  await init();
  const renderer = await Renderer.create(canvas);

  return {
    loadGcode: (text) => toLoadResult(renderer.load_gcode(text)),
    loadGcodeBegin: () => renderer.load_gcode_begin(),
    loadGcodeFeed: (text) => renderer.load_gcode_feed(text),
    loadGcodeEnd: () => toLoadResult(renderer.load_gcode_end()),
    render: () => renderer.render(),
    needsRedraw: () => renderer.needs_redraw(),
    resize: () => renderer.resize(),
    fit: () => renderer.fit(),
    pan: (dx, dy) => renderer.pan(dx, dy),
    zoom: (factor, x, y) => renderer.zoom(factor, x, y),
    orbit: (deltaYaw, deltaPitch) => renderer.orbit(deltaYaw, deltaPitch),
    setLineWidth: (width) => renderer.set_line_width(width),
    setExtrusionDiameter: (diameter) => renderer.set_extrusion_diameter(diameter),
    setZMax: (z) => renderer.set_z_max(z),
    setKindVisible: (kind, visible) => renderer.set_kind_visible(kind, visible),
    setColorByFeed: (byFeed) => renderer.set_color_by_feed(byFeed),
    setFeedRange: (min, max) => renderer.set_feed_range(min, max),
  };
}
