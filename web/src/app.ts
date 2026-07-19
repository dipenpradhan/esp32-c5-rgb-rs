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

export class App {
  private root: HTMLElement;
  private effects: Effect[];
  private previewAbort: AbortController | null = null;

  constructor(root: HTMLElement) {
    this.root = root;
    this.effects = [];
  }

  render(): void {
    this.root.innerHTML = `
      <h1>LED Effects</h1>

      <div class="card">
        <h2>Preview</h2>
        <div class="led-preview">
          <div class="led" id="led"></div>
          <div class="preview-info">
            <span id="stepLabel">No effects</span>
            <div class="preview-controls">
              <button class="btn-secondary btn-sm" id="stopPreview">Stop</button>
              <button class="btn-primary btn-sm" id="playPreview">Play</button>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <h2>Effects</h2>
        <div id="effectsList"></div>
        <div class="btn-row" style="margin-top:0.75rem">
          <button class="btn-primary" id="addBlink">+ Blink</button>
          <button class="btn-primary" id="addBlend">+ Blend</button>
        </div>
      </div>

      <div class="card">
        <h2>Config JSON</h2>
        <div class="preview-text" id="json">{ "effects": [] }</div>
        <div class="btn-row" style="margin-top:0.75rem">
          <button class="btn-secondary" id="copyJson">Copy JSON</button>
          <button class="btn-secondary" id="resetAll">Reset</button>
        </div>
      </div>
    `;

    document.getElementById("addBlink")!.addEventListener("click", () => this.addBlink());
    document.getElementById("addBlend")!.addEventListener("click", () => this.addBlend());
    document.getElementById("copyJson")!.addEventListener("click", () => this.copyJson());
    document.getElementById("resetAll")!.addEventListener("click", () => { this.effects = []; this.rebuild(); });
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
      el.innerHTML = `
        <div class="effect-header">
          <span class="effect-tag blink-tag">Blink</span>
          <span class="effect-info">Duration: ${effect.duration_ms}ms · ${effect.colors.length} colors</span>
          <button class="btn-remove">✕</button>
        </div>
        <div class="color-strip">
          ${effect.colors.map((c, ci) => `
            <div class="chip-wrap">
              <input type="color" class="color-pick" data-ci="${ci}" value="${rgbToHex(c)}">
            </div>
          `).join("")}
        </div>
        <div class="effect-controls">
          <div class="field-inline">
            <label>ms</label>
            <input type="number" class="dur-input" min="10" max="5000" value="${effect.duration_ms}">
          </div>
          <button class="btn-add-color btn-sm">+ Color</button>
          ${effect.colors.length > 1 ? `<button class="btn-remove-color btn-sm">- Color</button>` : ""}
        </div>
      `;

      // Remove effect
      el.querySelector(".btn-remove")!.addEventListener("click", () => this.removeEffect(i));

      // Duration
      const durInput = el.querySelector<HTMLInputElement>(".dur-input")!;
      durInput.addEventListener("change", () => {
        const val = Math.max(10, parseInt(durInput.value) || 300);
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
      el.innerHTML = `
        <div class="effect-header">
          <span class="effect-tag blend-tag">Blend</span>
          <span class="effect-info">${bf.steps} steps · ${bf.step_ms}ms/step</span>
          <button class="btn-remove">✕</button>
        </div>
        <div class="blend-row">
          <div class="color-block from-block">
            <label>From</label>
            <input type="color" class="color-pick" value="${rgbToHex(bf.from)}">
            <div class="color-swatch" style="background:rgb(${bf.from.join(",")})"></div>
          </div>
          <span class="blend-arrow">→</span>
          <div class="color-block to-block">
            <label>To</label>
            <input type="color" class="color-pick" value="${rgbToHex(bf.to)}">
            <div class="color-swatch" style="background:rgb(${bf.to.join(",")})"></div>
          </div>
        </div>
        <div class="effect-fields">
          <div class="field-pair">
            <label>Steps</label>
            <input type="number" class="num-input" min="1" max="200" value="${bf.steps}">
          </div>
          <div class="field-pair">
            <label>Step ms</label>
            <input type="number" class="ms-input" min="1" max="5000" value="${bf.step_ms}">
          </div>
        </div>
      `;

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
        bf.steps = Math.max(1, parseInt(el.querySelector<HTMLInputElement>(".num-input")!.value) || 20);
        (el.querySelector(".effect-info") as HTMLElement).textContent = `${bf.steps} steps · ${bf.step_ms}ms/step`;
        this.updateJson();
      });

      // Step ms
      el.querySelector<HTMLInputElement>(".ms-input")!.addEventListener("change", () => {
        bf.step_ms = Math.max(1, parseInt(el.querySelector<HTMLInputElement>(".ms-input")!.value) || 100);
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
    this.rebuild();
  }

  // ── Live preview ──────────────────────────────────────────────

  private startPreview(): void {
    this.stopPreview();
    if (this.effects.length === 0) return;

    this.previewAbort = new AbortController();
    const signal = this.previewAbort.signal;

    const led = document.getElementById("led")!;
    const label = document.getElementById("stepLabel")!;

    const run = async () => {
      while (!signal.aborted) {
        for (const effect of this.effects) {
          if (signal.aborted) break;

          if (effect.type === "blink") {
            for (const color of effect.colors) {
              if (signal.aborted) break;
              led.style.background = `rgb(${color[0]},${color[1]},${color[2]})`;
              led.style.boxShadow = `0 0 20px rgb(${color[0]},${color[1]},${color[2]})`;
              label.textContent = `Blink: ${rgbToHexShort(color)}`;
              await sleep(effect.duration_ms, signal);
            }
          } else {
            label.textContent = `Blend: ${rgbToHexShort(effect.from)} → ${rgbToHexShort(effect.to)}`;
            for (let step = 0; step <= effect.steps; step++) {
              if (signal.aborted) break;
              const t = step / effect.steps;
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