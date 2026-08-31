# configs

Two JSON files, both embedded into the firmware at compile time via
`include_str!`.

## effects.json

The list of LED effects the device runs. It is the compile-time config and
the runtime default. Two effect shapes exist:

- `blink` — cycles through `colors` (RGB triples), holding each for
  `duration_ms`.
- `blend` — interpolates from `from` to `to` over `steps` steps, `step_ms`
  apart.

```json
{
  "effects": [
    { "type": "blink", "colors": [[255,0,0],[0,255,0],[0,0,255]], "duration_ms": 300 },
    { "type": "blend", "from": [255,0,0], "to": [0,255,255], "steps": 20, "step_ms": 100 }
  ]
}
```

Note: editing this file can break `led-core` tests — several of them anchor to
its exact contents (see `led-core/README.md`). Run the host tests after
editing it.

## wifi.json

WiFi credentials, embedded into the firmware binary at compile time.
`wifi.json` is **not tracked** — copy `wifi.json.example` to `wifi.json`
and fill in real credentials:

```json
{ "ssid": "<your-ssid>", "password": "<your-password>" }
```

`wifi.json` is gitignored and must never be committed; only the
placeholder template (`wifi.json.example`) is committed.

### Warning: credentials are compiled into the firmware image

Because this file is embedded via `include_str!`, anyone who dumps the
board's flash can recover the credentials. That is the permanent risk and the
reason to treat any board you hand to someone else as disclosing that WiFi
password.

The real `wifi.json` is not tracked and is absent from the current git history
(the placeholder template `wifi.json.example` is the only committed form).
Earlier versions of this repo did commit the real file, so if you or a
collaborator have an older clone or remote that still contains it, treat that
password as compromised and rotate it.
