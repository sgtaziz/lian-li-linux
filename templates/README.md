# Template Catalog

Templates in this folder are published to the online browser in the GUI via
`default_templates.json`. Users install them from "Browse Online Templates"
on the LCD page.

## Layout

```
templates/
├── default_templates.json      # manifest (schema_version 1)
└── assets/
    └── <template-id>/
        ├── template.json       # the template definition
        ├── preview.png         # rendered preview (generate via render-preview)
        └── *.png / *.jpg       # any referenced background/widget assets
```

One folder per template, everything self-contained. Referenced assets are
paths relative to the folder (e.g. `"background.png"`), resolved at install
time to `~/.config/lianli/templates/<id>/<path>`.

## Authoring a new template

### 1. Pick an id and create the folder

```
templates/assets/my-template/
```

`id` must be unique across the catalog. Use lowercase-kebab.

### 2. Write `template.json`

Schema is `LcdTemplate` in [`crates/lianli-shared/src/template.rs`](../crates/lianli-shared/src/template.rs).
Minimum fields:

```json
{
  "id": "my-template",
  "name": "Display Name",
  "base_width": 480,
  "base_height": 480,
  "rotated": false,
  "background": { "type": "color", "rgb": [12, 15, 22, 255] },
  "widgets": [ ... ]
}
```

`background` can be `"color"`, `"image"` (with `"path"` relative to the
folder), and each widget is one of: `label`, `value_text`, `radial_gauge`,
`vertical_bar`, `horizontal_bar`, `speedometer`, `core_bars`, `image`,
`video`. See the existing `cooler` and `doublegauge` templates for realistic
examples covering most widget kinds.

#### Exporting one of your own templates

The quickest way to publish a template you've already built in the GUI
editor is to copy it out of your local config:

1. Open the template in the GUI editor and click **Copy JSON**. This
   produces a _portable_ export: every sensor-bearing widget whose source
   the editor recognizes (k10temp/coretemp CPU temps, amdgpu/radeon GPU
   temps, NVIDIA GPU temp/usage, AMD GPU usage, CPU/memory usage virtual
   sensors) has its `sensor_category` filled in automatically and its
   concrete `source` replaced with a neutral placeholder. Widgets whose
   source can't be generalized (custom shell commands, constants,
   motherboard/RAM/drive hwmon entries) are left alone.

   If you prefer a raw dump, you can instead grab the entry directly from
   `~/.config/lianli/lcd_templates.json` — each item in the top-level
   `"templates"` array is a complete `LcdTemplate` object. This copy will
   keep your machine-specific source paths, so you'll have to add
   `sensor_category` hints by hand (see the next section).
2. Save the JSON as `templates/assets/<your-id>/template.json`.
3. Change `id` to the catalog id (lowercase-kebab). `name` can stay as-is.
4. For every absolute asset path (e.g. `/home/you/.config/lianli/templates/cooler/background.png`),
   copy the referenced file into `templates/assets/<your-id>/` and rewrite
   the path to be just the filename (`"background.png"`). Check both
   `background.path` and any `widget.kind.path` (image/video widgets).
5. **Review the auto-inferred `sensor_category` hints.** Copy JSON's
   guesses are best-effort — open the file and make sure every
   sensor-bearing widget has the category you actually intended:
   - If a widget should be portable but no category was inferred (e.g.
     you picked a custom hwmon sensor the heuristic didn't recognize),
     add the right `sensor_category` yourself and leave the placeholder
     `source` in place.
   - If a widget is _intentionally_ bound to a specific command or a
     user-specific hwmon label and should not be rewritten on install,
     delete the `sensor_category` field and restore the concrete `source`.
     Be aware anyone installing the template won't have that sensor —
     the widget will read 0 for them.
6. Continue with steps 3–6 below to add the template to the manifest.

### 3. Use `sensor_category` hints for portability

Widgets that bind to sensors should put a placeholder `source` (any sensor
source that parses — `cpu_usage` is fine) AND a `sensor_category` hint at
the widget level:

```json
{
  "id": "value-temp",
  "sensor_category": "cpu_temp",
  "kind": {
    "type": "value_text",
    "source": { "type": "cpu_usage" },
    "unit": "°C",
    ...
  }
}
```

On install, the client walks widgets and rewrites each `source` to whatever
matches the user's actual hardware:

| `sensor_category` | Resolves to                                                               |
|-------------------|---------------------------------------------------------------------------|
| `cpu_temp`        | k10temp/coretemp `Tctl` / `Package id 0` if present                       |
| `gpu_temp`        | NVIDIA GPU temp, or amdgpu `edge` label                                   |
| `cpu_usage`       | virtual `cpu_usage` sensor (always available)                             |
| `gpu_usage`       | NVIDIA GPU utilization, or AMD sysfs `gpu_busy_percent` (amdgpu driver)   |
| `mem_usage`       | virtual `mem_usage` sensor                                                |

If a category has no match on the local machine, the widget keeps its
placeholder `source` — so make sure the placeholder would at least render
something sane.

Widgets that intentionally bind to a specific sensor (custom shell command,
a particular hwmon label) should omit `sensor_category`.

### 4. Generate `preview.png`

```bash
cargo run -p lianli-media --bin render-preview -- templates/assets/my-template
```

This loads `template.json`, injects canned mock sensor values (48°C temps,
28% usage, 24 randomized cores), renders one frame at the template's native
`base_width × base_height`, and writes `preview.png` next to it. Relative
asset paths are resolved by chdir'ing into the template folder, so reference
everything by filename.

Re-run whenever you change `template.json` or any asset.

### 5. Add the template to `default_templates.json`

Compute sha256 of every file the manifest will reference:

```bash
sha256sum templates/assets/my-template/template.json \
          templates/assets/my-template/preview.png \
          templates/assets/my-template/background.png
```

Add an entry:

```json
{
  "id": "my-template",
  "name": "My Template",
  "description": "One-line blurb shown in the card.",
  "author": "your-github-handle",
  "min_daemon_version": "0.3.3",
  "folder": "my-template",
  "template_file": "template.json",
  "template_sha256": "...",
  "preview": "preview.png",
  "preview_sha256": "...",
  "base_width": 480,
  "base_height": 480,
  "rotated": false,
  "files": [
    { "path": "background.png", "sha256": "..." }
  ]
}
```

`files[]` lists every non-template asset referenced by `template.json`. Do
**not** list `template.json` or `preview.png` in `files[]` — they have their
own top-level sha256 fields.

### 6. Bump `min_daemon_version` if you use new features

If your template uses a widget kind, sensor category, or background type
that didn't exist in an older release, set `min_daemon_version` to the
first release that supports it. The client filters out templates whose
`min_daemon_version` exceeds the running version, so older clients quietly
skip incompatible templates instead of failing to install.

## Local testing before publishing

You can test the full install flow against your local checkout without
pushing by temporarily pointing `CATALOG_BASE_URL` in
[`crates/lianli-shared/src/template_catalog.rs`](../crates/lianli-shared/src/template_catalog.rs)
at a `file://` URL:

```rust
const CATALOG_BASE_URL: &str = "file:///home/you/code/lian-li-linux/templates";
```

The catalog fetcher detects `file://` prefixes and reads from disk directly
(bypassing the reqwest HTTP client), so manifest + template.json + every
listed asset loads from your working copy. Rebuild, click "Browse Online"
in the GUI, and install. Revert the constant before committing.

## Publishing

Open a pull request against `main` with your new folder under
`templates/assets/<id>/` and your manifest entry in
`templates/default_templates.json`. **Prefix the PR title with
`[TEMPLATE]`** so template-only changes are easy to filter and review
separately from code changes — e.g. `[TEMPLATE] add cyberpunk CPU
dashboard`.

Include in the PR description:
- A short blurb about what the template shows
- A screenshot or link to the committed `preview.png`
- Which devices / resolutions you tested it on

Templates go live as soon as the PR lands on `main`. The client fetches
`default_templates.json` from
`raw.githubusercontent.com/.../main/templates/default_templates.json` each
time the user opens the browser — nothing is cached in the repo snapshot
shipped with the binary, so your template is available to every user
immediately on merge.
