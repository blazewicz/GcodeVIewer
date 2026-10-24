import type { RendererApi } from "./renderer";
import { colorByFeed } from "./state";

export interface SliderControl {
  type: "slider";
  id: string;
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  apply(value: number): void;
}

export interface ToggleControl {
  type: "toggle";
  id: string;
  label: string;
  value: boolean;
  swatch?: string;
  apply(value: boolean): void;
}

export type Control = SliderControl | ToggleControl;

export interface Controls {
  sliders: readonly SliderControl[];
  toggles: readonly ToggleControl[];
  kinds: readonly ToggleControl[];
}

// Palette indices match the shader's PALETTE array and the renderer's Kind
// enum, in order: rapid, feed, arc, spline.
const KINDS = [
  { name: "rapid", label: "G0 rapid", visible: false },
  { name: "feed", label: "G1 feed", visible: true },
  { name: "arc", label: "G2/G3 arc", visible: true },
  { name: "spline", label: "G5 spline", visible: true },
] as const;

export function createControls(api: RendererApi): Controls {
  return {
    sliders: [
      {
        type: "slider",
        id: "width",
        label: "Line width",
        min: 0.5,
        max: 6,
        step: 0.1,
        value: 1.6,
        apply: (value) => api.setLineWidth(value),
      },
      {
        type: "slider",
        id: "diameter",
        label: "Extrusion diameter",
        min: 0.05,
        max: 1,
        step: 0.05,
        value: 0.2,
        apply: (value) => api.setExtrusionDiameter(value),
      },
    ],
    toggles: [
      {
        type: "toggle",
        id: "color-feed",
        label: "Color by feedrate",
        value: false,
        apply: (value) => colorByFeed.set(value),
      },
    ],
    kinds: KINDS.map((kind, index): ToggleControl => ({
      type: "toggle",
      id: `kind-${kind.name}`,
      label: kind.label,
      value: kind.visible,
      swatch: kind.name,
      apply: (value) => api.setKindVisible(index, value),
    })),
  };
}

export function renderControls(root: HTMLElement, controls: readonly Control[]): void {
  for (const control of controls) {
    root.append(control.type === "slider" ? renderSlider(control) : renderToggle(control));
  }
}

function renderSlider(control: SliderControl): HTMLLabelElement {
  const label = document.createElement("label");
  label.className = "slider";
  label.append(control.label);

  const input = document.createElement("input");
  input.type = "range";
  input.id = control.id;
  input.min = String(control.min);
  input.max = String(control.max);
  input.step = String(control.step);
  input.value = String(control.value);
  input.addEventListener("input", () => control.apply(Number(input.value)));

  label.append(input);
  control.apply(control.value);
  return label;
}

function renderToggle(control: ToggleControl): HTMLLabelElement {
  const label = document.createElement("label");

  const input = document.createElement("input");
  input.type = "checkbox";
  input.id = control.id;
  input.checked = control.value;
  input.addEventListener("change", () => control.apply(input.checked));

  const toggle = document.createElement("span");
  toggle.className = "switch";

  label.append(input, toggle);
  if (control.swatch) {
    const swatch = document.createElement("span");
    swatch.className = `swatch ${control.swatch}`;
    label.append(swatch);
  }
  label.append(control.label);

  control.apply(control.value);
  return label;
}
