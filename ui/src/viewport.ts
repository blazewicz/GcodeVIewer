import type { RendererApi } from "./renderer";

type DragMode = "pan" | "rotate" | null;

function pixelRatio(): number {
  return window.devicePixelRatio || 1;
}

/**
 * Matches the canvas backing store to its CSS box and reports whether it
 * changed. Assigning `width`/`height` clears the canvas and resets the swap
 * chain, so the caller must redraw in the same task.
 */
export function syncCanvasSize(canvas: HTMLCanvasElement): boolean {
  const ratio = pixelRatio();
  const rect = canvas.getBoundingClientRect();
  const width = Math.max(1, Math.round(rect.width * ratio));
  const height = Math.max(1, Math.round(rect.height * ratio));
  if (width === canvas.width && height === canvas.height) {
    return false;
  }
  canvas.width = width;
  canvas.height = height;
  return true;
}

export function attachViewport(
  canvas: HTMLCanvasElement,
  api: RendererApi,
  onError: (message: string) => void,
): void {
  const frame = (): void => {
    // Resizing clears the canvas, so it has to happen here, immediately before
    // the draw: a ResizeObserver callback runs after the animation frame
    // callbacks but before paint, which would composite the cleared canvas.
    if (syncCanvasSize(canvas)) {
      api.resize();
    }
    try {
      api.render();
    } catch (error) {
      onError(`Render failed: ${String(error)}`);
      return;
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  let dragMode: DragMode = null;
  let last: [number, number] = [0, 0];

  canvas.addEventListener("mousedown", (event) => {
    if (event.button === 1) {
      event.preventDefault();
    }
  });

  canvas.addEventListener("pointerdown", (event) => {
    if (event.button === 1) {
      dragMode = "rotate";
      event.preventDefault();
    } else if (event.button === 0) {
      dragMode = event.ctrlKey || event.metaKey ? "rotate" : "pan";
    } else {
      return;
    }
    last = [event.clientX, event.clientY];
    canvas.setPointerCapture(event.pointerId);
  });

  canvas.addEventListener("pointermove", (event) => {
    if (!dragMode) {
      return;
    }
    const ratio = pixelRatio();
    const dx = event.clientX - last[0];
    const dy = event.clientY - last[1];
    if (dragMode === "pan") {
      api.pan(dx * ratio, dy * ratio);
    } else {
      // Orbit the camera: dragging right spins the model, dragging up tilts the
      // view toward the horizon and dragging down back toward the top.
      api.orbit(-dx * 0.01, dy * 0.01);
    }
    last = [event.clientX, event.clientY];
  });

  canvas.addEventListener("pointerup", (event) => {
    dragMode = null;
    canvas.releasePointerCapture(event.pointerId);
  });

  canvas.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const ratio = pixelRatio();
      api.zoom(
        Math.exp(-event.deltaY * 0.001),
        (event.clientX - rect.left) * ratio,
        (event.clientY - rect.top) * ratio,
      );
    },
    { passive: false },
  );
}
