// Model hint from a Bluetooth device name. Our own heuristic (not from the spec), based on the
// owner's tracker "FMB920_1234567". Mirrors bt_classic::model_hint in Rust.
export type ModelHint = { model: string; serial: string; source: "from Bluetooth name" };

const RE = /^([A-Z]{3}[A-Z0-9]{3,4})_(\d{6,8})$/;

export function nameModelHint(name: string | null | undefined): ModelHint | null {
  const m = name ? RE.exec(name) : null;
  return m ? { model: m[1], serial: m[2], source: "from Bluetooth name" } : null;
}
