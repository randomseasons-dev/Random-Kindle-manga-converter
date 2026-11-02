import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";

type Settings = {
  device: string;
  format: string;
  folderMode: string;
  rightToLeft: string;
  splitter: string;
  rotateFirst: string;
  scale: string;
  gamma: string;
  cropping: string;
  croppingPower: string;
  preserveMargin: string;
  chunkMb: string;
  kindlegen: string;
};

// defaults = the KCC setup this app replaces
const DEFAULTS: Settings = {
  device: "K11",
  format: "mobi",
  folderMode: "volumes",
  rightToLeft: "1",
  splitter: "2",
  rotateFirst: "1",
  scale: "upscale",
  gamma: "0",
  cropping: "2",
  croppingPower: "1.00",
  preserveMargin: "0",
  chunkMb: "300",
  kindlegen: "",
};

type Choice = [string, string];
type Field = {
  key: keyof Settings;
  label: string;
  tip: string;
  choices: Choice[];
  disabled?: (s: Settings) => boolean;
};

const range = (from: number, to: number, step: number, digits: number): Choice[] => {
  const out: Choice[] = [];
  for (let v = from; v <= to + 1e-9; v += step) out.push([v.toFixed(digits), v.toFixed(digits)]);
  return out;
};

const FIELDS: Field[] = [
  {
    key: "format",
    label: "Output format",
    tip: "MOBI: KindleGen's dual file (works on every Kindle). AZW3: only the modern KF8 part, a bit smaller.",
    choices: [["mobi", "MOBI"], ["azw3", "AZW3"]],
  },
  {
    key: "folderMode",
    label: "Folder layout",
    tip: "Volume folders: every subfolder of the folder you add becomes its own book (chapters inside a volume folder are merged). One book: everything in the folder becomes one book.",
    choices: [["volumes", "Each subfolder is a volume"], ["single", "Whole folder is one book"]],
  },
  {
    key: "rightToLeft",
    label: "Reading direction",
    tip: "Right-to-left for manga: pages turn the manga way and split spreads keep the right half first.",
    choices: [["1", "Right-to-left (manga)"], ["0", "Left-to-right (comics)"]],
  },
  {
    key: "splitter",
    label: "Spread splitter",
    tip: "What to do with double-page spreads. Split: cut into two pages. Rotate: show the whole spread turned sideways. Split and rotate: both (spread shown twice).",
    choices: [["0", "Split"], ["1", "Rotate"], ["2", "Split and rotate"]],
  },
  {
    key: "rotateFirst",
    label: "Rotated spread position",
    tip: "With 'Split and rotate': put the rotated spread before or after the two split pages.",
    choices: [["1", "Rotated page first"], ["0", "Rotated page last"]],
    disabled: (s) => s.splitter !== "2",
  },
  {
    key: "scale",
    label: "Stretch / Upscale",
    tip: "For pictures smaller than the Kindle screen. Off: leave them small. Upscale: enlarge, keep the shape. Stretch: fill the whole screen (shape is distorted).",
    choices: [["off", "Off"], ["upscale", "Upscale (keep shape)"], ["stretch", "Stretch (fill screen)"]],
  },
  {
    key: "gamma",
    label: "Gamma",
    tip: "Auto leaves tones as they are. Below 1.0 makes pages brighter, above 1.0 makes midtones darker (1.8 was KCC's old default).",
    choices: [["0", "Auto"], ...range(0.5, 2.5, 0.1, 1)],
  },
  {
    key: "cropping",
    label: "Cropping",
    tip: "Cut empty margins around the page. 'Margins + page numbers' also removes a small page number at the bottom.",
    choices: [["0", "Off"], ["1", "Margins"], ["2", "Margins + page numbers"]],
  },
  {
    key: "croppingPower",
    label: "Cropping power",
    tip: "Higher = more aggressive: cuts through light grey dust and scan noise. Lower = keeps anything slightly dark.",
    choices: range(0.25, 3, 0.25, 2),
    disabled: (s) => s.cropping === "0",
  },
  {
    key: "preserveMargin",
    label: "Preserve margin",
    tip: "After finding the crop, give back this percentage of the cut margin.",
    choices: ["0", "1", "2", "3", "4", "5", "6", "8", "10", "15", "20"].map((v): Choice => [v, `${v} %`]),
    disabled: (s) => s.cropping === "0",
  },
  {
    key: "chunkMb",
    label: "Chunk size",
    tip: "A book bigger than this is saved as several files (Part 01, Part 02…). More than 300 MB can be slow on older Kindles.",
    choices: ["100", "150", "200", "250", "300", "350", "400", "450", "500"].map((v): Choice => [v, `${v} MB`]),
  },
];

type LogLine = { level: string; text: string };

type Device = { id: string; name: string; group: string; w: number; h: number; color: boolean; kf7: boolean };

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem("kmc-settings");
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    /* ignore */
  }
  return DEFAULTS;
}

export default function App() {
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [inputs, setInputs] = useState<string[]>([]);
  const [log, setLog] = useState<LogLine[]>([]);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [kgPath, setKgPath] = useState<string | null>(null);
  const [devices, setDevices] = useState<Device[]>([]);
  const logRef = useRef<HTMLDivElement>(null);
  const dev = devices.find((d) => d.id === settings.device);
  const groups = [...new Set(devices.map((d) => d.group))];

  useEffect(() => {
    try {
      localStorage.setItem("kmc-settings", JSON.stringify(settings));
    } catch {
      /* ignore */
    }
  }, [settings]);

  const checkKindlegen = async (custom = settings.kindlegen) => {
    const p = await invoke<string | null>("find_kindlegen", { custom: custom || null });
    setKgPath(p);
  };

  useEffect(() => {
    checkKindlegen();
    invoke<Device[]>("list_devices").then(setDevices);
    const un = [
      listen<LogLine>("log", (e) => setLog((l) => [...l, e.payload])),
      listen<{ done: number; total: number }>("progress", (e) => setProgress(e.payload)),
      listen("finished", () => setRunning(false)),
      getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === "drop") addInputs(e.payload.paths);
      }),
    ];
    return () => {
      un.forEach((p) => p.then((f) => f()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  const addInputs = (paths: string[]) =>
    setInputs((cur) => [...cur, ...paths.filter((p) => !cur.includes(p))]);

  const pickFolders = async () => {
    const r = await open({ directory: true, multiple: true, title: "Add folder(s)" });
    if (r) addInputs(Array.isArray(r) ? r : [r]);
  };

  const browseKindlegen = async () => {
    const r = await open({
      multiple: false,
      title: "Find kindlegen.exe",
      filters: [{ name: "kindlegen", extensions: ["exe"] }],
    });
    if (typeof r === "string") {
      setSettings((s) => ({ ...s, kindlegen: r }));
      checkKindlegen(r);
    }
  };

  const convert = async () => {
    if (!inputs.length) return;
    setLog([]);
    setProgress({ done: 0, total: 0 });
    setRunning(true);
    const s = settings;
    try {
      await invoke("start_convert", {
        inputs,
        opts: {
          device: s.device,
          format: dev?.kf7 ? "mobi" : s.format,
          folderMode: s.folderMode,
          rightToLeft: s.rightToLeft === "1",
          splitter: Number(s.splitter),
          rotateFirst: s.rotateFirst === "1",
          scale: s.scale,
          gamma: Number(s.gamma),
          cropping: Number(s.cropping),
          croppingPower: Number(s.croppingPower),
          preserveMargin: Number(s.preserveMargin),
          chunkMb: Number(s.chunkMb),
          kindlegen: s.kindlegen || null,
        },
      });
    } catch (e) {
      setLog((l) => [...l, { level: "error", text: String(e) }]);
      setRunning(false);
    }
  };

  const set = (key: keyof Settings, value: string) => setSettings((s) => ({ ...s, [key]: value }));
  const pct = progress.total ? Math.round((progress.done / progress.total) * 100) : 0;

  return (
    <div className="app">
      <section className="inputs">
        <div className="row">
          <button onClick={pickFolders} disabled={running}>Add folder(s)</button>
          <button onClick={() => setInputs([])} disabled={running || !inputs.length}>Clear list</button>
          <span className="hint">or drag folders onto the window</span>
        </div>
        <ul className="input-list">
          {inputs.length === 0 && <li className="empty">No folders yet. Add a folder of pictures, or a series folder with one subfolder per volume.</li>}
          {inputs.map((p) => (
            <li key={p}>
              <span title={p}>{p}</span>
              <button className="x" disabled={running} onClick={() => setInputs((c) => c.filter((x) => x !== p))} title="Remove">×</button>
            </li>
          ))}
        </ul>
      </section>

      <section className="settings">
        <label className="field device" title="Pages are resized for this Kindle's screen. Colour Kindles keep colour pages in colour.">
          <span>
            Device
            {dev && ` · ${dev.w} × ${dev.h}${dev.color ? " · colour" : ""}${dev.kf7 ? " · MOBI only" : ""}`}
          </span>
          <select value={settings.device} disabled={running} onChange={(e) => set("device", e.target.value)}>
            {groups.map((g) => (
              <optgroup key={g} label={g}>
                {devices
                  .filter((d) => d.group === g)
                  .map((d) => (
                    <option key={d.id} value={d.id}>{d.name}</option>
                  ))}
              </optgroup>
            ))}
          </select>
        </label>
        {FIELDS.map((f) => (
          <label key={f.key} className="field" title={f.tip}>
            <span>{f.label}</span>
            <select
              value={f.key === "format" && dev?.kf7 ? "mobi" : settings[f.key]}
              disabled={running || f.disabled?.(settings)}
              onChange={(e) => set(f.key, e.target.value)}
            >
              {f.choices.map(([v, l]) => (
                <option key={v} value={v} disabled={f.key === "format" && v === "azw3" && dev?.kf7}>
                  {l}
                </option>
              ))}
            </select>
          </label>
        ))}
        <button className="reset" disabled={running} onClick={() => setSettings({ ...DEFAULTS, kindlegen: settings.kindlegen })}>
          Reset to defaults
        </button>
      </section>

      <section className={`kindlegen ${kgPath ? "found" : "missing"}`}>
        {kgPath ? (
          <span>KindleGen: <code title={kgPath}>{kgPath}</code></span>
        ) : (
          <span>KindleGen not found: install Kindle Previewer 3 (free, from Amazon), then press "Check again".</span>
        )}
        <div className="row">
          {!kgPath && (
            <button onClick={() => openUrl("https://www.amazon.com/Kindle-Previewer/b?ie=UTF8&node=21381691011")}>
              Get Kindle Previewer
            </button>
          )}
          <button onClick={browseKindlegen}>Browse…</button>
          <button onClick={() => checkKindlegen()}>Check again</button>
        </div>
      </section>

      <section className="actions">
        {running ? (
          <button className="primary stop" onClick={() => invoke("stop_convert")}>Stop</button>
        ) : (
          <button className="primary" onClick={convert} disabled={!inputs.length || !kgPath}>Convert</button>
        )}
        <div className="bar"><div style={{ width: `${pct}%` }} /></div>
        <span className="pct">{progress.total ? `${progress.done}/${progress.total} pictures` : ""}</span>
      </section>

      <div className="log" ref={logRef}>
        {log.map((l, i) => {
          const saved = l.text.startsWith("Saved: ") ? l.text.slice(7) : null;
          return (
            <div key={i} className={`line ${l.level}`}>
              {l.text}
              {saved && (
                <button
                  className="link"
                  onClick={() => invoke("open_folder", { path: saved.replace(/[\\/][^\\/]*$/, "") })}
                >
                  open folder
                </button>
              )}
            </div>
          );
        })}
      </div>
      <footer>Output is saved next to the folder it was made from · made by RandomSeasons</footer>
    </div>
  );
}
