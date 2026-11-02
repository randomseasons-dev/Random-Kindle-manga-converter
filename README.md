# Kindle Manga Converter

A simple Windows app that turns folders of manga pictures into **MOBI** or **AZW3** books for the Kindle 11. It's a slimmed-down replacement for Kindle Comic Converter (KCC): only the options that matter are kept, and each one is a dropdown.

## Before you start

The final step uses Amazon's **KindleGen**, which comes with the free **Kindle Previewer 3**. Install Kindle Previewer and the app finds KindleGen automatically. You can also point the app to `kindlegen.exe` with **Browse…**.

## How to use

1. Click **Add folder(s)**, or drag folders onto the window.
   - A folder of pictures becomes one book.
   - A series folder with one subfolder per volume becomes one book per volume (chapter folders inside a volume are merged).
2. Pick your options and click **Convert**.
3. Each book is saved next to the folder it was made from. Click **open folder** in the log to jump to it.

## Options

| Option | Choices |
|---|---|
| **Output format** | MOBI (KindleGen's dual file) · AZW3 (KF8 only) |
| **Folder layout** | Each subfolder is a volume · Whole folder is one book |
| **Reading direction** | Right-to-left (manga) · Left-to-right (comics) |
| **Spread splitter** | Split · Rotate · Split and rotate (for double-page spreads) |
| **Rotated spread position** | Rotated page first · Rotated page last (with Split and rotate) |
| **Stretch / Upscale** | Off · Upscale (keep shape) · Stretch (fill screen), for pictures smaller than the screen |
| **Gamma** | Auto · 0.5 – 2.5 (below 1 = brighter, above 1 = darker midtones) |
| **Cropping** | Off · Margins · Margins + page numbers |
| **Cropping power** | 0.25 – 3.00 (higher = more aggressive) |
| **Preserve margin** | 0 – 20 % of the cut margin given back |
| **Chunk size** | 100 – 500 MB. A bigger book is saved as Part 01, Part 02… |

Every page is also converted to grayscale, contrast-stretched and resized to the Kindle 11 screen (1072 × 1448).

## Build

Requires Node.js and Rust.

```
npm install
npx tauri build
```

The app and installers are written to `src-tauri/target/release/`.

## Credits

Page processing follows [Kindle Comic Converter](https://github.com/ciromattia/kcc). The AZW3 extraction is based on KindleUnpack's `mobi_split`.

## Changelog

- **1.0** — first release.

made by RandomSeasons
