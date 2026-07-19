export class App {
  private root: HTMLElement;
  private effectType = "blink";
  private r = 255;
  private g = 0;
  private b = 0;
  private durationMs = 300;
  private configJson: string;

  constructor(root: HTMLElement) {
    this.root = root;
    this.configJson = this.buildConfig();
  }

  render(): void {
    this.root.innerHTML = `
      <h1>LED Effects</h1>

      <div class="card">
        <h2>Preview</h2>
        <div class="led-preview">
          <div class="led" id="led" style="background: rgb(${this.r},${this.g},${this.b})"></div>
        </div>
      </div>

      <div class="card">
        <h2>Effect</h2>
        <div class="form-group">
          <label>Type</label>
          <select id="effectType">
            <option value="blink" ${this.effectType === "blink" ? "selected" : ""}>Blink</option>
            <option value="blend" ${this.effectType === "blend" ? "selected" : ""}>Blend</option>
          </select>
        </div>

        <div class="form-group">
          <label>Color</label>
          <div class="color-row">
            <input type="color" id="colorPicker" value="${this.hexColor()}">
            <input type="number" id="rVal" min="0" max="255" value="${this.r}">
            <input type="number" id="gVal" min="0" max="255" value="${this.g}">
            <input type="number" id="bVal" min="0" max="255" value="${this.b}">
          </div>
        </div>

        <div class="form-group">
          <label>Duration (ms)</label>
          <input type="number" id="duration" min="10" max="5000" value="${this.durationMs}">
        </div>

        <div class="btn-row">
          <button class="btn-primary" id="apply">Apply</button>
          <button class="btn-secondary" id="preview">Preview</button>
        </div>
      </div>

      <div class="card">
        <h2>Config JSON</h2>
        <div class="preview-text" id="json">${this.configJson}</div>
      </div>
    `;

    this.bindEvents();
  }

  private bindEvents(): void {
    const effectTypeEl = document.getElementById("effectType") as HTMLSelectElement;
    const colorPickerEl = document.getElementById("colorPicker") as HTMLInputElement;
    const rEl = document.getElementById("rVal") as HTMLInputElement;
    const gEl = document.getElementById("gVal") as HTMLInputElement;
    const bEl = document.getElementById("bVal") as HTMLInputElement;
    const durationEl = document.getElementById("duration") as HTMLInputElement;
    const applyBtn = document.getElementById("apply")!;
    const previewBtn = document.getElementById("preview")!;
    const ledEl = document.getElementById("led")!;

    effectTypeEl.addEventListener("change", (e) => {
      this.effectType = (e.target as HTMLSelectElement).value;
      this.updateJson();
    });

    colorPickerEl.addEventListener("input", (e) => {
      const hex = (e.target as HTMLInputElement).value;
      [this.r, this.g, this.b] = this.hexToRgb(hex);
      rEl.value = String(this.r);
      gEl.value = String(this.g);
      bEl.value = String(this.b);
      ledEl.style.background = `rgb(${this.r},${this.g},${this.b})`;
      this.updateJson();
    });

    [rEl, gEl, bEl].forEach((input, i) => {
      input.addEventListener("input", () => {
        const val = Math.min(255, Math.max(0, parseInt(input.value) || 0));
        input.value = String(val);
        [this.r, this.g, this.b][i] = val;
        colorPickerEl.value = this.hexColor();
        ledEl.style.background = `rgb(${this.r},${this.g},${this.b})`;
        this.updateJson();
      });
    });

    durationEl.addEventListener("input", () => {
      this.durationMs = Math.max(10, parseInt(durationEl.value) || 300);
      this.updateJson();
    });

    applyBtn.addEventListener("click", () => {
      console.log("Config applied:", this.configJson);
      alert("Config generated! Copy it to configs/effects.json and rebuild.");
    });

    previewBtn.addEventListener("click", () => {
      this.animatePreview(ledEl);
    });
  }

  private updateJson(): void {
    this.configJson = this.buildConfig();
    const jsonEl = document.getElementById("json")!;
    jsonEl.textContent = this.configJson;
  }

  private buildConfig(): string {
    const color = [this.r, this.g, this.b];
    if (this.effectType === "blink") {
      return JSON.stringify(
        {
          effects: [
            {
              type: "blink",
              colors: [color, [this.g, this.r, this.b]],
              duration_ms: this.durationMs,
            },
          ],
        },
        null,
        2
      );
    } else {
      return JSON.stringify(
        {
          effects: [
            {
              type: "blend",
              from: color,
              to: [this.g, this.r, this.b],
              steps: 20,
              step_ms: this.durationMs,
            },
          ],
        },
        null,
        2
      );
    }
  }

  private hexColor(): string {
    return `#${this.r.toString(16).padStart(2, "0")}${this.g.toString(16).padStart(2, "0")}${this.b.toString(16).padStart(2, "0")}`;
  }

  private hexToRgb(hex: string): [number, number, number] {
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return [r, g, b];
  }

  private animatePreview(led: HTMLElement): void {
    const [r, g, b] = [this.r, this.g, this.b];
    const [r2, g2, b2] = [this.g, this.r, this.b];
    const steps = 10;

    led.style.animation = "none";
    void led.offsetWidth;

    let step = 0;
    const id = setInterval(() => {
      const frac = step / steps;
      led.style.background = `rgb(${Math.round(r + (r2 - r) * frac)},${Math.round(g + (g2 - g) * frac)},${Math.round(b + (b2 - b) * frac)})`;
      step++;
      if (step > steps) {
        clearInterval(id);
      }
    }, this.durationMs / steps);
  }
}