// The device injects a per-config auth token into the page it serves (at
// serve time) so the browser can authenticate POST /config. It is not a
// standard DOM property, so augment Window with a declaration rather than
// casting — this is type-only and erased at build.
declare global {
  interface Window {
    CONFIG_TOKEN?: string;
  }
}

type Color = [number, number, number];

interface BlinkEffect {
  type: "blink";
  colors: Color[];
  duration_ms: number;
}

interface BlendEffect {
  type: "blend";
  from: Color;
  to: Color;
  steps: number;
  step_ms: number;
}

type Effect = BlinkEffect | BlendEffect;

// Limits shared with the firmware's serde/semantic model. Keep in sync —
// the frontend must accept exactly what the device accepts, and clamp or
// drop everything else before it reaches a fetch or the preview loop.
const MAX_EFFECTS = 32;
const MAX_COLORS = 64;
const MIN_STEPS = 1;
const MAX_STEPS = 64;
const MAX_MS = 60000;

function isColor(v: unknown): v is Color {
  return Array.isArray(v)
    && v.length === 3
    && v.every((c) => Number.isInteger(c) && c >= 0 && c <= 255);
}

// Narrow `unknown` (device JSON is untrusted input — the device's serde
// model used to be the only thing rejecting malformed configs) into a
// conforming Effect[]. Non-conforming effects are dropped; conforming
// values outside firmware limits are clamped. Returns [] on any shape
// mismatch, so callers can fall back to defaults.
function sanitizeEffects(raw: unknown): Effect[] {
  if (!Array.isArray(raw)) return [];
  const out: Effect[] = [];
  for (const item of raw.slice(0, MAX_EFFECTS)) {
    if (!item || typeof item !== "object") continue;
    const e = item as Record<string, unknown>;
    if (e.type === "blink") {
      if (!Array.isArray(e.colors)) continue;
      const colors = e.colors.slice(0, MAX_COLORS).filter(isColor);
      const duration = Number(e.duration_ms);
      if (!Number.isInteger(duration) || duration <= 0) continue;
      out.push({ type: "blink", colors, duration_ms: Math.min(duration, MAX_MS) });
    } else if (e.type === "blend") {
      if (!isColor(e.from) || !isColor(e.to)) continue;
      const steps = Number(e.steps);
      const stepMs = Number(e.step_ms);
      if (!Number.isInteger(steps) || steps < MIN_STEPS || !Number.isInteger(stepMs) || stepMs <= 0) continue;
      out.push({ type: "blend", from: e.from, to: e.to, steps: Math.min(steps, MAX_STEPS), step_ms: Math.min(stepMs, MAX_MS) });
    }
    // any other type is dropped: the preview branches only on "blink"/"blend",
    // so an unknown value must never reach it as a "blend"
  }
  return out;
}

// fetch with a hard timeout: a half-hung device must not leave the UI
// stuck mid-save (disabled button, "Saving…" text) for the life of the page.
async function fetchWithTimeout(url: string, init: RequestInit, ms: number): Promise<Response> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), ms);
  try {
    return await fetch(url, { ...init, signal: ctrl.signal });
  } finally {
    clearTimeout(timer);
  }
}

export class App {
  private root: HTMLElement;
  private effects: Effect[];
  private previewAbort: AbortController | null = null;

  constructor(root: HTMLElement) {
    this.root = root;
    this.effects = [];
  }

  render(): void {
    // Templates are compact single lines: HTML is whitespace-insensitive
    // between block elements, so dropping indent/newlines costs nothing at
    // render time but shaves ~700B off the inlined bundle (TX-buffer budget).
    this.root.innerHTML =
      `<h1>LED Effects</h1>` +
      `<div class="card"><h2>Preview</h2><div class="led-preview"><div class="led" id="led" role="img" aria-label="LED color preview"></div><div class="preview-info"><span id="stepLabel" aria-live="polite">No effects</span><div class="preview-controls"><button class="btn-secondary btn-sm" id="stopPreview">Stop</button><button class="btn-primary btn-sm" id="playPreview">Play</button></div></div></div></div>` +
      `<div class="card"><h2>Effects</h2><div id="effectsList"></div><div class="btn-row" style="margin-top:0.75rem"><button class="btn-primary" id="addBlink">+ Blink</button><button class="btn-primary" id="addBlend">+ Blend</button></div></div>` +
      `<div class="card"><h2>Config JSON</h2><div class="preview-text" id="json">{ "effects": [] }</div><div class="btn-row" style="margin-top:0.75rem"><button class="btn-primary" id="saveToDevice">Save to Device</button><button class="btn-secondary" id="copyJson">Copy JSON</button><button class="btn-secondary" id="resetAll">Reset</button></div><div id="status" role="status"></div></div>`;

    document.getElementById("addBlink")!.addEventListener("click", () => this.addBlink());
    document.getElementById("addBlend")!.addEventListener("click", () => this.addBlend());
    document.getElementById("copyJson")!.addEventListener("click", () => this.copyJson());
    document.getElementById("resetAll")!.addEventListener("click", () => { this.effects = []; this.stopPreview(); this.rebuild(); });
    document.getElementById("saveToDevice")!.addEventListener("click", () => this.saveToDevice());
    document.getElementById("playPreview")!.addEventListener("click", () => this.startPreview());
    document.getElementById("stopPreview")!.addEventListener("click", () => this.stopPreview());

    this.updateEmptyMsg();
    this.updateJson();
  }

  private rebuild(): void {
    this.renderEffectsList();
    this.updateJson();
  }

  private renderEffectsList(): void {
    const container = document.getElementById("effectsList")!;
    container.innerHTML = "";

    if (this.effects.length === 0) {
      this.updateEmptyMsg();
      return;
    }

    this.effects.forEach((effect, i) => {
      container.appendChild(this.createEffectElement(effect, i));
    });
  }

  private updateEmptyMsg(): void {
    const container = document.getElementById("effectsList")!;
    if (this.effects.length === 0) {
      container.innerHTML = `<p class="empty-msg">Click + Blink or + Blend below to add effects</p>`;
    }
  }

  private createEffectElement(effect: Effect, i: number): HTMLElement {
    const el = document.createElement("div");
    el.className = "effect-item";

    if (effect.type === "blink") {
      el.innerHTML =
        `<div class="effect-header"><span class="effect-tag blink-tag">Blink</span><span class="effect-info">Duration: ${effect.duration_ms}ms · ${effect.colors.length} colors</span><button class="btn-remove" aria-label="Remove effect">✕</button></div>` +
        `<div class="color-strip">${effect.colors.map((c, ci) =>
          `<div class="chip-wrap"><input type="color" id="c${i}_${ci}" class="color-pick" data-ci="${ci}" value="${rgbToHex(c)}"></div>`
        ).join("")}</div>` +
        `<div class="effect-controls"><div class="field-inline"><label for="d${i}">ms</label><input type="number" id="d${i}" class="dur-input" min="10" max="${MAX_MS}" value="${effect.duration_ms}"></div><button class="btn-add-color btn-sm">+ Color</button>${effect.colors.length > 1 ? `<button class="btn-remove-color btn-sm">- Color</button>` : ""}</div>`;

      // Color pickers get an accessible name from their value (no visible text
      // in this compact strip)
      el.querySelectorAll<HTMLInputElement>(".color-pick").forEach((inp) => {
        inp.setAttribute("aria-label", `Color ${inp.value}`);
      });

      // Remove effect
      el.querySelector(".btn-remove")!.addEventListener("click", () => this.removeEffect(i));

      // Duration
      const durInput = el.querySelector<HTMLInputElement>(".dur-input")!;
      durInput.addEventListener("change", () => {
        const val = Math.min(MAX_MS, Math.max(10, parseInt(durInput.value) || 300));
        (this.effects[i] as BlinkEffect).duration_ms = val;
        (el.querySelector(".effect-info") as HTMLElement).textContent = `Duration: ${val}ms · ${(this.effects[i] as BlinkEffect).colors.length} colors`;
        this.updateJson();
      });

      // Color pickers — always visible, change fires immediately
      el.querySelectorAll<HTMLInputElement>(".color-pick").forEach((inp) => {
        inp.addEventListener("input", () => {
          const ci = parseInt(inp.dataset.ci!);
          (this.effects[i] as BlinkEffect).colors[ci] = hexToRgb(inp.value);
          this.updateJson();
        });
      });

      // Add color
      el.querySelector(".btn-add-color")!.addEventListener("click", () => {
        const be = this.effects[i] as BlinkEffect;
        be.colors.push([255, 0, 0]);
        this.rebuild();
      });

      // Remove color
      const rmBtn = el.querySelector(".btn-remove-color");
      if (rmBtn) rmBtn.addEventListener("click", () => {
        (this.effects[i] as BlinkEffect).colors.pop();
        this.rebuild();
      });

    } else {
      const bf = effect as BlendEffect;
      el.innerHTML =
        `<div class="effect-header"><span class="effect-tag blend-tag">Blend</span><span class="effect-info">${bf.steps} steps · ${bf.step_ms}ms/step</span><button class="btn-remove" aria-label="Remove effect">✕</button></div>` +
        `<div class="blend-row"><div class="color-block from-block"><label for="f${i}">From</label><input type="color" id="f${i}" class="color-pick" value="${rgbToHex(bf.from)}"><div class="color-swatch" style="background:rgb(${bf.from.join(",")})"></div></div><span class="blend-arrow">→</span><div class="color-block to-block"><label for="t${i}">To</label><input type="color" id="t${i}" class="color-pick" value="${rgbToHex(bf.to)}"><div class="color-swatch" style="background:rgb(${bf.to.join(",")})"></div></div></div>` +
        `<div class="effect-fields"><div class="field-pair"><label for="s${i}">Steps</label><input type="number" id="s${i}" class="num-input" min="1" max="${MAX_STEPS}" value="${bf.steps}"></div><div class="field-pair"><label for="m${i}">Step ms</label><input type="number" id="m${i}" class="ms-input" min="1" max="${MAX_MS}" value="${bf.step_ms}"></div></div>`;

      // Remove effect
      el.querySelector(".btn-remove")!.addEventListener("click", () => this.removeEffect(i));

      // From color
      const fromPick = el.querySelector<HTMLInputElement>(".from-block .color-pick")!;
      const fromSwatch = el.querySelector<HTMLDivElement>(".from-block .color-swatch")!;
      fromPick.addEventListener("input", () => {
        bf.from = hexToRgb(fromPick.value);
        fromSwatch.style.background = `rgb(${bf.from.join(",")})`;
        this.updateJson();
      });

      // To color
      const toPick = el.querySelector<HTMLInputElement>(".to-block .color-pick")!;
      const toSwatch = el.querySelector<HTMLDivElement>(".to-block .color-swatch")!;
      toPick.addEventListener("input", () => {
        bf.to = hexToRgb(toPick.value);
        toSwatch.style.background = `rgb(${bf.to.join(",")})`;
        this.updateJson();
      });

      // Steps
      el.querySelector<HTMLInputElement>(".num-input")!.addEventListener("change", () => {
        bf.steps = Math.min(MAX_STEPS, Math.max(1, parseInt(el.querySelector<HTMLInputElement>(".num-input")!.value) || 20));
        (el.querySelector(".effect-info") as HTMLElement).textContent = `${bf.steps} steps · ${bf.step_ms}ms/step`;
        this.updateJson();
      });

      // Step ms
      el.querySelector<HTMLInputElement>(".ms-input")!.addEventListener("change", () => {
        bf.step_ms = Math.min(MAX_MS, Math.max(1, parseInt(el.querySelector<HTMLInputElement>(".ms-input")!.value) || 100));
        (el.querySelector(".effect-info") as HTMLElement).textContent = `${bf.steps} steps · ${bf.step_ms}ms/step`;
        this.updateJson();
      });
    }

    return el;
  }

  private updateJson(): void {
    const json = JSON.stringify({ effects: this.effects }, null, 2);
    document.getElementById("json")!.textContent = json;
  }

  private copyJson(): void {
    navigator.clipboard.writeText(JSON.stringify({ effects: this.effects }, null, 2));
    const btn = document.getElementById("copyJson")!;
    const orig = btn.textContent!;
    btn.textContent = "Copied!";
    setTimeout(() => { btn.textContent = orig; }, 1500);
  }

  // ── Device sync (two-way config) ──────────────────────────────

  private setStatus(msg: string, kind: "ok" | "err" | "" = ""): void {
    const el = document.getElementById("status");
    if (!el) return;
    el.textContent = msg;
    el.className = kind ? `status ${kind}` : "status";
  }

  private async saveToDevice(): Promise<void> {
    const btn = document.getElementById("saveToDevice");
    // Re-validate before POST: the UI clamps on edit, but the array could
    // have been mutated by another code path since the last render.
    this.effects = sanitizeEffects(this.effects);
    const body = JSON.stringify({ effects: this.effects });
    // POST /config is now auth-gated by the device; it rejects any POST
    // without the token it injected at serve time (window.CONFIG_TOKEN).
    // Standalone (no device) leaves that undefined — send no token header
    // and let the 403 surface through the normal error path below.
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (window.CONFIG_TOKEN !== undefined) headers["X-Config-Token"] = window.CONFIG_TOKEN;
    if (btn) (btn as HTMLButtonElement).disabled = true;
    this.setStatus("Saving to device…");
    try {
      const res = await fetchWithTimeout("/config", {
        method: "POST",
        headers,
        body,
      }, 5000);
      const data = await res.json().catch(() => ({}));
      if (res.ok && data.ok) {
        this.setStatus("✓ Saved — LED updated", "ok");
      } else {
        this.setStatus(`✗ ${data.error || "HTTP " + res.status}`, "err");
      }
    } catch (e) {
      this.setStatus(e instanceof DOMException && e.name === "AbortError"
        ? "✗ Save timed out (no device response)"
        : "✗ Save failed (device offline?)", "err");
    } finally {
      if (btn) (btn as HTMLButtonElement).disabled = false;
    }
  }

  async loadFromDevice(): Promise<void> {
    try {
      const res = await fetchWithTimeout("/config", {}, 5000);
      if (!res.ok) throw new Error("HTTP " + res.status);
      const data = await res.json();
      const raw = (data && typeof data === "object" && "effects" in data) ? (data as { effects?: unknown }).effects : undefined;
      const clean = sanitizeEffects(raw);
      if (clean.length > 0) {
        this.effects = clean;
        this.stopPreview(); // never animate a list that is about to be replaced
        this.rebuild();
        this.setStatus(`Loaded ${clean.length} effects from device`, "ok");
      } else {
        this.setStatus("Device has no effects yet");
      }
    } catch (e) {
      // Running standalone (not served by the device) — keep whatever is in the UI.
      this.setStatus(e instanceof DOMException && e.name === "AbortError"
        ? "⚠ Load timed out — showing defaults"
        : "Standalone mode (not served by device)");
    }
  }

  private addBlink(): void {
    this.effects.push({ type: "blink", colors: [[255, 0, 0]], duration_ms: 300 });
    const container = document.getElementById("effectsList")!;
    if (!container.querySelector(".empty-msg")) {
      container.appendChild(this.createEffectElement(this.effects[this.effects.length - 1], this.effects.length - 1));
    } else {
      this.rebuild();
    }
    this.updateJson();
  }

  private addBlend(): void {
    this.effects.push({ type: "blend", from: [255, 0, 0], to: [0, 255, 0], steps: 20, step_ms: 100 });
    const container = document.getElementById("effectsList")!;
    if (!container.querySelector(".empty-msg")) {
      container.appendChild(this.createEffectElement(this.effects[this.effects.length - 1], this.effects.length - 1));
    } else {
      this.rebuild();
    }
    this.updateJson();
  }

  private removeEffect(i: number): void {
    this.effects.splice(i, 1);
    this.stopPreview(); // splice invalidates indices a running loop is iterating
    this.rebuild();
  }

  // ── Live preview ──────────────────────────────────────────────

  private startPreview(): void {
    this.stopPreview();
    if (this.effects.length === 0) return;

    this.previewAbort = new AbortController();
    const signal = this.previewAbort.signal;

    // Snapshot: remove/reset/load mutate this.effects underneath a running
    // loop, and animating a stale list that no longer matches the UI is
    // exactly the staleness this preview is meant to show.
    const list = this.effects.slice();

    const led = document.getElementById("led")!;
    const label = document.getElementById("stepLabel")!;

    const run = async () => {
      while (!signal.aborted) {
        for (const effect of list) {
          if (signal.aborted) break;

          if (effect.type === "blink") {
            for (const color of effect.colors) {
              if (signal.aborted) break;
              led.style.background = `rgb(${color[0]},${color[1]},${color[2]})`;
              led.style.boxShadow = `0 0 20px rgb(${color[0]},${color[1]},${color[2]})`;
              label.textContent = `Blink: ${rgbToHexShort(color)}`;
              await sleep(effect.duration_ms, signal);
            }
          } else if (effect.type === "blend") {
            // explicit branch (no else): an unknown type is skipped, not
            // misread as a blend's .from/.to/.steps
            label.textContent = `Blend: ${rgbToHexShort(effect.from)} → ${rgbToHexShort(effect.to)}`;
            for (let step = 0; step <= effect.steps; step++) {
              if (signal.aborted) break;
              const t = effect.steps > 0 ? step / effect.steps : 0; // NaN t makes invalid rgb() CSS
              const color = lerpColor(effect.from, effect.to, t);
              led.style.background = `rgb(${color[0]},${color[1]},${color[2]})`;
              led.style.boxShadow = `0 0 20px rgb(${color[0]},${color[1]},${color[2]})`;
              await sleep(effect.step_ms, signal);
            }
          }
        }
      }
    };

    run();
  }

  private stopPreview(): void {
    this.previewAbort?.abort();
    this.previewAbort = null;
    const led = document.getElementById("led");
    if (led) led.style.boxShadow = "none";
    const label = document.getElementById("stepLabel");
    if (label) label.textContent = "Stopped";
  }
}

// ── Helpers ────────────────────────────────────────────────────────

function rgbToHex(rgb: Color): string {
  return `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

function hexToRgb(hex: string): Color {
  return [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ] as Color;
}

function rgbToHexShort(rgb: Color): string {
  return `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;
}

function lerpColor(from: Color, to: Color, t: number): Color {
  return [
    Math.round(from[0] + (to[0] - from[0]) * t),
    Math.round(from[1] + (to[1] - from[1]) * t),
    Math.round(from[2] + (to[2] - from[2]) * t),
  ] as Color;
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) { resolve(); return; }
    const id = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => { clearTimeout(id); resolve(); }, { once: true });
  });
}
